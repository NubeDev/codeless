use codeless_rpc::{
    AppendAssistantMessageArgs, AppendAssistantMessageResult, CancelAssistantActionArgs,
    CancelAssistantActionResult, ConfirmAssistantActionArgs, ConfirmAssistantActionResult,
    CreateAssistantThreadArgs, DeleteAssistantThreadArgs, ListAssistantMessagesArgs,
    ListAssistantMessagesResult, ListAssistantThreadsArgs, ListAssistantThreadsResult,
    SetAssistantThreadModeArgs, SetAssistantThreadModeResult, UploadAssistantAttachmentArgs,
    UploadAssistantAttachmentResult,
};
use codeless_types::AssistantThread;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_list_assistant_threads(
    state: State<'_, AppState>,
    args: ListAssistantThreadsArgs,
) -> CommandResult<ListAssistantThreadsResult> {
    Ok(state.rpc.list_assistant_threads(args).await?)
}

#[tauri::command]
pub async fn rpc_create_assistant_thread(
    state: State<'_, AppState>,
    args: CreateAssistantThreadArgs,
) -> CommandResult<AssistantThread> {
    Ok(state.rpc.create_assistant_thread(args).await?)
}

#[tauri::command]
pub async fn rpc_delete_assistant_thread(
    state: State<'_, AppState>,
    args: DeleteAssistantThreadArgs,
) -> CommandResult<()> {
    Ok(state.rpc.delete_assistant_thread(args).await?)
}

#[tauri::command]
pub async fn rpc_set_assistant_thread_mode(
    state: State<'_, AppState>,
    args: SetAssistantThreadModeArgs,
) -> CommandResult<SetAssistantThreadModeResult> {
    Ok(state.rpc.set_assistant_thread_mode(args).await?)
}

#[tauri::command]
pub async fn rpc_upload_assistant_attachment(
    state: State<'_, AppState>,
    args: UploadAssistantAttachmentArgs,
) -> CommandResult<UploadAssistantAttachmentResult> {
    Ok(state.rpc.upload_assistant_attachment(args).await?)
}

#[tauri::command]
pub async fn rpc_list_assistant_messages(
    state: State<'_, AppState>,
    args: ListAssistantMessagesArgs,
) -> CommandResult<ListAssistantMessagesResult> {
    Ok(state.rpc.list_assistant_messages(args).await?)
}

#[tauri::command]
pub async fn rpc_append_assistant_message(
    state: State<'_, AppState>,
    args: AppendAssistantMessageArgs,
) -> CommandResult<AppendAssistantMessageResult> {
    Ok(state.rpc.append_assistant_message(args).await?)
}

#[tauri::command]
pub async fn rpc_confirm_assistant_action(
    state: State<'_, AppState>,
    args: ConfirmAssistantActionArgs,
) -> CommandResult<ConfirmAssistantActionResult> {
    Ok(state.rpc.confirm_assistant_action(args).await?)
}

#[tauri::command]
pub async fn rpc_cancel_assistant_action(
    state: State<'_, AppState>,
    args: CancelAssistantActionArgs,
) -> CommandResult<CancelAssistantActionResult> {
    Ok(state.rpc.cancel_assistant_action(args).await?)
}
