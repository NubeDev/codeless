use std::collections::HashMap;
use std::sync::Arc;

use codeless_rpc::{RpcServer, ServerInfo};
use tokio_util::sync::CancellationToken;

/// Channel-id to cancellation-token map. `rpc_subscribe` inserts;
/// `rpc_unsubscribe` and the forwarder's drop-guard both remove.
pub type SubscriptionMap = parking_lot::Mutex<HashMap<u32, CancellationToken>>;

/// Shared state managed by Tauri and injected into every command via
/// `tauri::State<'_, AppState>`. Mirrors `codeless-server::AppState`
/// but replaces the HTTP auth layer with the Tauri IPC trust boundary
/// (same-process, same-user — no token needed).
pub struct AppState {
    pub rpc: Arc<dyn RpcServer>,
    pub server_info: Arc<ServerInfo>,
    pub subs: Arc<SubscriptionMap>,
}
