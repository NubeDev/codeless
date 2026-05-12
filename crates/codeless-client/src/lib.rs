//! SSE + REST client for `codeless-server`. The CLI's `--core` mode
//! is the first consumer; the future mobile shells (Phase 6) reach
//! this crate transitively because it is iOS- and Android-safe (no
//! `std::process`, no host-only crates in the dependency graph).
//!
//! Wire contract is locked by
//! `ui/codeless-ui/src/lib/rpc/http-sse-client.ts` — the browser
//! `HttpSseClient` and this crate must serialise the same bytes for
//! the same call. `codeless-server::routes::map_err` is the canonical
//! HTTP-status → `RpcError` mapping; this client decodes the same
//! way so the two transports surface identical errors.

mod http_client;
mod sse;

pub use http_client::{ClientError, HttpRpcClient, HttpRpcClientConfig};
