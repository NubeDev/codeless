use codeless_rpc::{
    AttachWorkspaceArgs, AttachWorkspaceResult, DetachWorkspaceArgs, ListWorkspacesResult,
    ValidateWorkspacePathArgs, ValidateWorkspacePathResult,
};
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_attach_workspace(
    state: State<'_, AppState>,
    args: AttachWorkspaceArgs,
) -> CommandResult<AttachWorkspaceResult> {
    Ok(state.rpc.attach_workspace(args).await?)
}

#[tauri::command]
pub async fn rpc_detach_workspace(
    state: State<'_, AppState>,
    args: DetachWorkspaceArgs,
) -> CommandResult<()> {
    Ok(state.rpc.detach_workspace(args).await?)
}

#[tauri::command]
pub async fn rpc_list_workspaces(
    state: State<'_, AppState>,
) -> CommandResult<ListWorkspacesResult> {
    Ok(state.rpc.list_workspaces().await?)
}

#[tauri::command]
pub async fn rpc_validate_workspace_path(
    state: State<'_, AppState>,
    args: ValidateWorkspacePathArgs,
) -> CommandResult<ValidateWorkspacePathResult> {
    Ok(state.rpc.validate_workspace_path(args).await?)
}
