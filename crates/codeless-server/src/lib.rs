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

mod auth;
mod routes;
mod sse;

use std::sync::Arc;

use axum::Router;
use codeless_rpc::RpcServer;

pub use auth::TokenLoadError;

/// Shared handler state. Cloned cheaply: both fields are `Arc`-shaped
/// (the trait object behind `RpcServer` is wrapped in `Arc`; the
/// bearer string is wrapped in `Arc<str>` so cloning the state does
/// not duplicate the secret across every request).
#[derive(Clone)]
pub struct AppState {
    pub rpc: Arc<dyn RpcServer>,
    pub bearer_token: Arc<str>,
}

impl AppState {
    pub fn new(rpc: Arc<dyn RpcServer>, bearer_token: impl Into<Arc<str>>) -> Self {
        Self {
            rpc,
            bearer_token: bearer_token.into(),
        }
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
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    on_bound(local);
    let app = build_router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
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
