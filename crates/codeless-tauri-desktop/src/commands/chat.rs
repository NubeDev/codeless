use codeless_rpc::{
    AgentChatArgs, AgentChatResult, CancelChatTaskArgs, StopActiveArgs, StopActiveResult,
    UploadChatAttachmentArgs, UploadChatAttachmentResult,
};
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_agent_chat(
    state: State<'_, AppState>,
    args: AgentChatArgs,
) -> CommandResult<AgentChatResult> {
    Ok(state.rpc.agent_chat(args).await?)
}

#[tauri::command]
pub async fn rpc_upload_chat_attachment(
    state: State<'_, AppState>,
    args: UploadChatAttachmentArgs,
) -> CommandResult<UploadChatAttachmentResult> {
    Ok(state.rpc.upload_chat_attachment(args).await?)
}

#[tauri::command]
pub async fn rpc_cancel_chat_task(
    state: State<'_, AppState>,
    args: CancelChatTaskArgs,
) -> CommandResult<()> {
    Ok(state.rpc.cancel_chat_task(args).await?)
}

#[tauri::command]
pub async fn rpc_stop_active(
    state: State<'_, AppState>,
    args: StopActiveArgs,
) -> CommandResult<StopActiveResult> {
    Ok(state.rpc.stop_active(args).await?)
}
