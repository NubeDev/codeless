//! Route-level coverage for the plugin REST surface — `GET /plugins`
//! (bearer-gated listing) and `GET /plugins/<id>/ui/*`
//! (`ServeDir`, no auth). Lives in its own integration-test file so
//! the existing `routes.rs` test fixture stays unchanged while the
//! new endpoints are exercised end-to-end through `tower::oneshot`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use codeless_runtime::InProcessRpc;
use codeless_server::{
    build_router, AppState, PluginCatalog, PluginCatalogEntry, PluginListResponse,
};
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789";

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

async fn make_app(catalog: Option<Arc<PluginCatalog>>) -> axum::Router {
    let rpc = Arc::new(InProcessRpc::new().await.expect("rpc init"));
    let mut state = AppState::new(rpc, TOKEN);
    if let Some(c) = catalog {
        state = state.with_plugins(c);
    }
    build_router(state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_plugins_404s_without_a_catalog() {
    // With no catalog attached the server compiles without the
    // sub-router, so the listing route is not registered at all.
    // This is the only safe shape for the existing route table that
    // does not have plugin support compiled in.
    let app = make_app(None).await;
    let resp = app.oneshot(get("/plugins", Some(TOKEN))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_plugins_requires_bearer() {
    let catalog = Arc::new(PluginCatalog::from_entries([PluginCatalogEntry {
        id: "notes".into(),
        version: "0.1.0".into(),
        contributes_ui: false,
        slots: Vec::new(),
        ui_dir: None,
    }]));
    let app = make_app(Some(catalog)).await;
    let resp = app.oneshot(get("/plugins", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_plugins_returns_catalog_projection() {
    let catalog = Arc::new(PluginCatalog::from_entries([
        PluginCatalogEntry {
            id: "notes".into(),
            version: "0.1.0".into(),
            contributes_ui: true,
            slots: vec!["assistant-panel".into()],
            // The path itself is not exposed in JSON; it only matters
            // for the ServeDir mount tested below.
            ui_dir: None,
        },
        PluginCatalogEntry {
            id: "headless".into(),
            version: "0.2.0".into(),
            contributes_ui: false,
            slots: Vec::new(),
            ui_dir: None,
        },
    ]));
    let app = make_app(Some(catalog)).await;
    let resp = app
        .clone()
        .oneshot(get("/plugins", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body: PluginListResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.plugins.len(), 2);
    assert_eq!(body.plugins[0].id, "notes");
    assert_eq!(body.plugins[0].version, "0.1.0");
    assert!(body.plugins[0].contributes_ui);
    assert_eq!(body.plugins[0].slots, vec!["assistant-panel"]);
    assert_eq!(body.plugins[1].id, "headless");
    assert!(!body.plugins[1].contributes_ui);
    assert!(body.plugins[1].slots.is_empty());

    // The `ui_dir` field must not leak through the JSON projection —
    // it is a host-local filesystem path the host shell has no need
    // for. Spot-check by re-parsing the bytes as a loose Value.
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let first = &raw["plugins"][0];
    assert!(first.get("ui_dir").is_none(), "ui_dir leaked into JSON");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_servedir_serves_bundle_without_auth() {
    let tmp = TempDir::new().unwrap();
    let ui_dir = tmp.path().join("notes/ui");
    std::fs::create_dir_all(&ui_dir).unwrap();
    std::fs::write(ui_dir.join("mf-manifest.json"), r#"{"name":"notes"}"#).unwrap();

    let catalog = Arc::new(PluginCatalog::from_entries([PluginCatalogEntry {
        id: "notes".into(),
        version: "0.1.0".into(),
        contributes_ui: true,
        slots: vec!["assistant-panel".into()],
        ui_dir: Some(ui_dir.clone()),
    }]));
    let app = make_app(Some(catalog)).await;

    // No bearer header at all — matches the host UI bundle posture.
    let resp = app
        .clone()
        .oneshot(get("/plugins/notes/ui/mf-manifest.json", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(&bytes[..], br#"{"name":"notes"}"#);

    // A path that does not exist under the ServeDir 404s — proves the
    // mount is rooted at the plugin's `ui/` directory and not at a
    // wider scope (e.g. the plugin root or the tempdir).
    let resp = app
        .oneshot(get("/plugins/notes/ui/missing.js", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_servedir_is_scoped_to_the_plugin_id() {
    let tmp = TempDir::new().unwrap();
    let ui_dir = tmp.path().join("notes/ui");
    std::fs::create_dir_all(&ui_dir).unwrap();
    std::fs::write(ui_dir.join("mf-manifest.json"), b"{}").unwrap();

    let catalog = Arc::new(PluginCatalog::from_entries([PluginCatalogEntry {
        id: "notes".into(),
        version: "0.1.0".into(),
        contributes_ui: true,
        slots: Vec::new(),
        ui_dir: Some(ui_dir),
    }]));
    let app = make_app(Some(catalog)).await;

    // A different plugin id must not resolve into the notes ServeDir.
    let resp = app
        .oneshot(get("/plugins/other/ui/mf-manifest.json", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
