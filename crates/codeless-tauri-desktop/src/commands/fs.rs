use codeless_rpc::{
    FsCreateDirArgs, FsCreateFileArgs, FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs,
    FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs,
};
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_fs_read_dir(
    state: State<'_, AppState>,
    args: FsReadDirArgs,
) -> CommandResult<FsReadDirResult> {
    Ok(state.rpc.fs_read_dir(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_read_file(
    state: State<'_, AppState>,
    args: FsReadFileArgs,
) -> CommandResult<FsReadFileResult> {
    Ok(state.rpc.fs_read_file(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_write_file(
    state: State<'_, AppState>,
    args: FsWriteFileArgs,
) -> CommandResult<()> {
    Ok(state.rpc.fs_write_file(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_stat(
    state: State<'_, AppState>,
    args: FsStatArgs,
) -> CommandResult<FsStatResult> {
    Ok(state.rpc.fs_stat(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_cwd(state: State<'_, AppState>) -> CommandResult<FsCwdResult> {
    Ok(state.rpc.fs_cwd().await?)
}

#[tauri::command]
pub async fn rpc_fs_create_file(
    state: State<'_, AppState>,
    args: FsCreateFileArgs,
) -> CommandResult<()> {
    Ok(state.rpc.fs_create_file(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_create_dir(
    state: State<'_, AppState>,
    args: FsCreateDirArgs,
) -> CommandResult<()> {
    Ok(state.rpc.fs_create_dir(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_move(state: State<'_, AppState>, args: FsMoveArgs) -> CommandResult<()> {
    Ok(state.rpc.fs_move(args).await?)
}

#[tauri::command]
pub async fn rpc_fs_delete(state: State<'_, AppState>, args: FsDeleteArgs) -> CommandResult<()> {
    Ok(state.rpc.fs_delete(args).await?)
}
