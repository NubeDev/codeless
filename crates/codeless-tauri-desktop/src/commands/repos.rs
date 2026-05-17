use codeless_rpc::{AddRepoArgs, ListReposResult, RemoveRepoArgs};
use codeless_types::Repo;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_add_repo(state: State<'_, AppState>, args: AddRepoArgs) -> CommandResult<Repo> {
    Ok(state.rpc.add_repo(args).await?)
}

#[tauri::command]
pub async fn rpc_remove_repo(
    state: State<'_, AppState>,
    args: RemoveRepoArgs,
) -> CommandResult<()> {
    Ok(state.rpc.remove_repo(args).await?)
}

#[tauri::command]
pub async fn rpc_list_repos(state: State<'_, AppState>) -> CommandResult<ListReposResult> {
    Ok(state.rpc.list_repos().await?)
}
