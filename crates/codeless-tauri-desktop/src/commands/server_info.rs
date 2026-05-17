use codeless_rpc::ServerInfo;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn rpc_server_info(state: State<'_, AppState>) -> Result<ServerInfo, ()> {
    Ok((*state.server_info).clone())
}
