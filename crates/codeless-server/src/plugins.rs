//! Plugin REST surface — `GET /plugins` (listing) and
//! `GET /plugins/<id>/ui/*` (Module Federation bundle ServeDir).
//!
//! The listing is the projection a host-shell uses to discover which
//! plugins are loaded and which of them ship a UI bundle. The ServeDir
//! mounts are what the host shell's MF runtime then fetches the
//! `mf-manifest.json` + chunk files from. See
//! [`DOCS/plugins/PLUGIN-UI-FEDERATION.md` § Server wiring]
//! (`../../../DOCS/plugins/PLUGIN-UI-FEDERATION.md`).
//!
//! The catalog is opt-in: a server with `AppState::plugins == None`
//! does not register either route. This keeps the existing unit tests
//! and the bare `codeless serve` path (no plugins compiled in) free of
//! the surface. Tests that need the routes construct a catalog
//! explicitly via [`PluginCatalog::from_entries`].

use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

use crate::AppState;

/// One plugin's row in the `GET /plugins` response and the on-disk
/// directory the ServeDir for `/plugins/<id>/ui/*` reads from.
///
/// `slots` is the list of host slot ids the plugin contributes to
/// (`assistant-panel`, `tool-result:<tool_id>`, …) as resolved from
/// the manifest. The field is populated by the catalog builder, not
/// re-derived here — the manifest's `[contributes.ui]` block is the
/// authoritative source and only the CLI / runtime can read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCatalogEntry {
    pub id: String,
    pub version: String,
    /// True when `ui_dir` is set *and* a `mf-manifest.json` file
    /// lives inside it. The catalog builder is expected to check
    /// this; the server takes the value verbatim. False means the
    /// host shell will skip MF registration for the plugin.
    pub contributes_ui: bool,
    /// Host slot ids the plugin's exposes contribute to. Empty when
    /// the plugin ships no UI or its manifest declares no exposes.
    #[serde(default)]
    pub slots: Vec<String>,
    /// Filesystem path of the plugin directory's `ui/` subdir. When
    /// set the router mounts a ServeDir at `/plugins/<id>/ui`; when
    /// `None`, only the listing row is rendered. Not exposed in the
    /// JSON projection — operators inspecting the listing do not care
    /// about server-local paths.
    #[serde(skip)]
    pub ui_dir: Option<PathBuf>,
}

/// The full catalog the server reflects through `GET /plugins`. The
/// host (CLI / test harness) builds this from its plugin registry
/// once at startup and hands it to [`AppState::with_plugins`]; the
/// server keeps it in an `Arc` so router rebuilds and request handlers
/// share the same allocation.
///
/// The catalog is immutable for the server's lifetime. Hot-reload of
/// plugin bundles is intentionally out of scope: the substrate model
/// is static-link at startup, and refreshing the bundle requires a
/// server restart. This is consistent with the rest of the substrate
/// (registration table is `&'static`, manifests parse once).
#[derive(Debug, Default)]
pub struct PluginCatalog {
    entries: Vec<PluginCatalogEntry>,
}

impl PluginCatalog {
    /// Build a catalog from an iterator of entries. Order is preserved
    /// so a host that wants stable listing order (e.g. lexicographic)
    /// can sort upstream and the server will not reshuffle.
    pub fn from_entries(entries: impl IntoIterator<Item = PluginCatalogEntry>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    pub fn entries(&self) -> &[PluginCatalogEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// JSON shape of the `GET /plugins` response. Mirrors the doc:
/// `[{ id, version, contributes_ui, slots: [...] }]` wrapped in a
/// `{"plugins": [...]}` envelope so future fields (paging cursor,
/// `total`, etc.) can be added without breaking the wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginListResponse {
    pub plugins: Vec<PluginListRow>,
}

/// Projection of one [`PluginCatalogEntry`] that omits the host-local
/// `ui_dir`. The host shell joins the bundle path itself
/// (`/plugins/<id>/ui/mf-manifest.json`) and never sees the absolute
/// filesystem location.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginListRow {
    pub id: String,
    pub version: String,
    pub contributes_ui: bool,
    pub slots: Vec<String>,
}

impl From<&PluginCatalogEntry> for PluginListRow {
    fn from(e: &PluginCatalogEntry) -> Self {
        Self {
            id: e.id.clone(),
            version: e.version.clone(),
            contributes_ui: e.contributes_ui,
            slots: e.slots.clone(),
        }
    }
}

/// Bearer-gated handler for `GET /plugins`. Mounted by
/// [`bearer_routes`]; falls back to an empty list when the catalog is
/// `None` so the route never returns `500` on a misconfigured server.
/// The middleware in [`crate::auth::bearer_layer`] gates the route,
/// matching the posture of every other authenticated `/rpc/*` endpoint
/// — discovery of which plugins are installed is operator-facing
/// information and must not leak before the operator presents a
/// token.
pub(crate) async fn list_handler(State(state): State<AppState>) -> Json<PluginListResponse> {
    let plugins = state
        .plugins
        .as_ref()
        .map(|c| c.entries().iter().map(PluginListRow::from).collect())
        .unwrap_or_default();
    Json(PluginListResponse { plugins })
}

/// Build the bearer-gated portion of the plugin surface: just the
/// `GET /plugins` listing. Returns `None` when no catalog is wired so
/// the caller can skip merging the sub-router entirely.
pub(crate) fn bearer_routes(state: &AppState) -> Option<Router<AppState>> {
    state.plugins.as_ref()?;
    Some(Router::new().route("/plugins", get(list_handler)))
}

/// Build the un-authenticated `ServeDir` mounts: one
/// `/plugins/<id>/ui` per entry whose `ui_dir` is set. Returns `None`
/// when the catalog is absent or every entry lacks a `ui_dir`.
///
/// No bearer check sits in front of the ServeDir: the bundle is
/// equivalent in sensitivity to the host's own UI bundle (the rest of
/// `codeless/ui/`), which is also served as static files behind only
/// the loopback bind. The bearer guard on the listing keeps discovery
/// of *which* plugins exist server-side; the bundle bytes themselves
/// are no more sensitive than any other JS the browser loads.
/// See PLUGIN-UI-FEDERATION.md § Server wiring.
pub(crate) fn ui_routes(state: &AppState) -> Option<Router<AppState>> {
    let catalog = state.plugins.as_ref()?;
    let mut router = Router::new();
    let mut mounted = false;
    for entry in catalog.entries() {
        let Some(dir) = entry.ui_dir.as_ref() else {
            continue;
        };
        let prefix = format!("/plugins/{}/ui", entry.id);
        router = router.nest_service(&prefix, ServeDir::new(dir));
        mounted = true;
    }
    mounted.then_some(router)
}

/// Convenience wrapper around [`Arc::new`] so call sites that already
/// import `PluginCatalog` do not need a second use-line.
pub fn shared(catalog: PluginCatalog) -> Arc<PluginCatalog> {
    Arc::new(catalog)
}
