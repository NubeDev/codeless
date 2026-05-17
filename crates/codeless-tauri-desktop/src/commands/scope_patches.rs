use codeless_rpc::{
    ApproveScopePatchArgs, EditScopePatchArgs, ListProposedPatchesArgs, ListProposedPatchesResult,
    RejectScopePatchArgs, RevertScopePatchArgs, RevertScopePatchResult, ScopePatchActionResult,
    SetJobPolicyArgs,
};
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_approve_scope_patch(
    state: State<'_, AppState>,
    args: ApproveScopePatchArgs,
) -> CommandResult<ScopePatchActionResult> {
    Ok(state.rpc.approve_scope_patch(args).await?)
}

#[tauri::command]
pub async fn rpc_reject_scope_patch(
    state: State<'_, AppState>,
    args: RejectScopePatchArgs,
) -> CommandResult<ScopePatchActionResult> {
    Ok(state.rpc.reject_scope_patch(args).await?)
}

#[tauri::command]
pub async fn rpc_edit_scope_patch(
    state: State<'_, AppState>,
    args: EditScopePatchArgs,
) -> CommandResult<ScopePatchActionResult> {
    Ok(state.rpc.edit_scope_patch(args).await?)
}

#[tauri::command]
pub async fn rpc_revert_scope_patch(
    state: State<'_, AppState>,
    args: RevertScopePatchArgs,
) -> CommandResult<RevertScopePatchResult> {
    Ok(state.rpc.revert_scope_patch(args).await?)
}

#[tauri::command]
pub async fn rpc_list_proposed_patches(
    state: State<'_, AppState>,
    args: ListProposedPatchesArgs,
) -> CommandResult<ListProposedPatchesResult> {
    Ok(state.rpc.list_proposed_patches(args).await?)
}

#[tauri::command]
pub async fn rpc_set_job_policy(
    state: State<'_, AppState>,
    args: SetJobPolicyArgs,
) -> CommandResult<()> {
    Ok(state.rpc.set_job_policy(args).await?)
}
