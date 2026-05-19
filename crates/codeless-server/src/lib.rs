//! Hosted HTTP surface for `codeless-runtime`. The browser shell and
//! (future) mobile shells reach the runtime through this server; the
//! Tauri desktop shell bypasses it via in-process IPC instead.
//!
//! The wire contract is fixed by
//! `codeless/ui/codeless-ui/src/lib/rpc/http-sse-client.ts` — every
//! method is `POST /rpc/<method>` with the args struct as the JSON
//! body, and the event stream is `GET /events` as SSE with the auth
//! token passed via `?token=` (browsers cannot set headers on
//! `EventSource`). Diverging from that contract silently breaks the
//! browser; the TS file is the spec, not this one.
//!
//! Authentication is a single bearer token shared by every client —
//! single-tenant MVP per SCOPE.md R5. Phase 7 swaps this for OIDC.

mod ai_ui;
mod auth;
pub mod plugins;
mod routes;
mod sse;

use std::sync::Arc;

use axum::Router;
use codeless_rpc::{RpcServer, ServerInfo};

pub use auth::TokenLoadError;
pub use plugins::{PluginCatalog, PluginCatalogEntry, PluginListResponse, PluginListRow};

/// How the server gates incoming requests. `Required` means every
/// `/rpc/*` request must carry the bearer token and every `/events`
/// connection must include it in the query string. `Open` skips the
/// check entirely — only the CLI's `codeless serve` opts into this
/// for loopback binds, where the trust boundary is already the
/// same-user-same-host process (SCOPE.md R5).
#[derive(Clone, Debug)]
pub enum AuthMode {
    Required { token: Arc<str> },
    Open,
}

impl AuthMode {
    /// Helper for callers that always operated on a token string;
    /// keeps the older test setup paths short.
    pub fn required(token: impl Into<Arc<str>>) -> Self {
        AuthMode::Required {
            token: token.into(),
        }
    }
}

/// Shared handler state. Cloned cheaply: both fields are `Arc`-shaped
/// (the trait object behind `RpcServer` is wrapped in `Arc`; the
/// bearer-token allocation behind `AuthMode::Required` is also
/// `Arc<str>` so cloning the state does not duplicate the secret
/// across every request).
#[derive(Clone)]
pub struct AppState {
    pub rpc: Arc<dyn RpcServer>,
    pub auth: AuthMode,
    /// Snapshot served unauthenticated at `GET /server/info`. The UI
    /// reads it once on boot to populate the runner dropdown and to
    /// decide whether to render the demo-only path. Wrapped in `Arc`
    /// so cloning the state across handler invocations stays cheap;
    /// the contents are immutable for the server's lifetime.
    pub server_info: Arc<ServerInfo>,
    /// Optional ai-ui surface. When present, the router mounts
    /// `/api/ai-ui/{chat,push,events,skills,components}`; when `None`,
    /// those routes are not registered and the server is unchanged.
    /// Built from `codeless_ai_ui::CodelessProvider` plus a skill
    /// registry and a component manifest — see
    /// `codeless-cli/src/serve.rs` for the production wiring.
    pub ai_ui: Option<ai_ui_core::AiUiState>,
    /// Plugin catalog projection. When present, the router exposes
    /// `GET /plugins` (bearer-gated) and `GET /plugins/<id>/ui/*`
    /// (ServeDir, no auth — same posture as the host UI bundle).
    /// `None` skips both registrations entirely so a server compiled
    /// without plugin support is byte-for-byte identical on the wire.
    /// Wrapped in `Arc` to share the allocation across cloned states.
    pub plugins: Option<Arc<PluginCatalog>>,
}

impl AppState {
    /// Build a state with the legacy "bearer required" mode. The
    /// `Into<Arc<str>>` bound preserves the call sites that pass a
    /// borrowed string or `String` directly.
    pub fn new(rpc: Arc<dyn RpcServer>, bearer_token: impl Into<Arc<str>>) -> Self {
        Self {
            rpc,
            auth: AuthMode::required(bearer_token),
            server_info: Arc::new(ServerInfo::default()),
            ai_ui: None,
            plugins: None,
        }
    }

    /// Build a state with auth disabled. Used by the CLI on loopback
    /// binds; tests can also reach this constructor when they want to
    /// bypass the bearer header.
    pub fn open(rpc: Arc<dyn RpcServer>) -> Self {
        Self {
            rpc,
            auth: AuthMode::Open,
            server_info: Arc::new(ServerInfo::default()),
            ai_ui: None,
            plugins: None,
        }
    }

    /// Replace the `/server/info` payload. Returns `self` for builder-
    /// style chaining at the CLI call site, which is the only producer
    /// of a non-default `ServerInfo`. Tests that do not care about the
    /// snapshot can stick with the constructors above.
    pub fn with_server_info(mut self, info: ServerInfo) -> Self {
        self.server_info = Arc::new(info);
        self
    }

    /// Attach an `ai-ui` surface. The router will then mount the
    /// `/api/ai-ui/*` routes; without this call those routes are
    /// absent. Cheap to clone (`AiUiState` is `Arc`-backed internally).
    pub fn with_ai_ui(mut self, ai_ui: ai_ui_core::AiUiState) -> Self {
        self.ai_ui = Some(ai_ui);
        self
    }

    /// Attach a [`PluginCatalog`]. The router will then expose
    /// `GET /plugins` and a per-plugin `GET /plugins/<id>/ui/*`
    /// ServeDir mount for every entry whose `ui_dir` is set. Cheap to
    /// clone — the catalog is shared through `Arc`.
    pub fn with_plugins(mut self, catalog: Arc<PluginCatalog>) -> Self {
        self.plugins = Some(catalog);
        self
    }
}

/// Build the axum router. The caller is responsible for binding a
/// listener and driving `axum::serve` — keeping the binary entry
/// point out of this crate's API surface lets the test suite exercise
/// every route without opening a port.
pub fn build_router(state: AppState) -> Router {
    routes::router(state)
}

/// Bind to `addr` and serve until SIGINT (Ctrl-C). The bound socket
/// address is reported via the `on_bound` callback before `axum::serve`
/// is awaited so callers — both the production CLI and the integration
/// tests — can discover an ephemeral port (`127.0.0.1:0`) without
/// racing the server.
pub async fn serve_with_shutdown<F>(
    addr: std::net::SocketAddr,
    state: AppState,
    on_bound: F,
) -> std::io::Result<()>
where
    F: FnOnce(std::net::SocketAddr),
{
    serve_with_extra_shutdown(addr, state, on_bound, std::future::pending::<()>()).await
}

/// Like `serve_with_shutdown` but accepts an extra shutdown future the
/// caller wires up — typically the runtime's `RestartTrigger` so a
/// successful `restart_server` RPC drains the listener without going
/// through Ctrl-C. Either future resolving wins; the other is dropped.
pub async fn serve_with_extra_shutdown<F, S>(
    addr: std::net::SocketAddr,
    state: AppState,
    on_bound: F,
    extra: S,
) -> std::io::Result<()>
where
    F: FnOnce(std::net::SocketAddr),
    S: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    on_bound(local);
    let app = build_router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = extra => {}
            }
        })
        .await
}

/// Token key used in the secrets file. Defined here (and not on the
/// CLI side) so the library can produce the same "run `codeless serve
/// --init-token` first" hint that the binary will print.
pub const TOKEN_SECRET_KEY: &str = "core_bearer_token";

/// Read the bearer token from a `SecretStore`. Returns
/// `TokenLoadError::Missing` when the key is absent so the CLI can
/// print a specific hint rather than a generic "internal error".
pub fn load_bearer_token(
    store: &codeless_adapters_host::SecretStore,
) -> Result<String, TokenLoadError> {
    auth::load_bearer_token(store)
}
