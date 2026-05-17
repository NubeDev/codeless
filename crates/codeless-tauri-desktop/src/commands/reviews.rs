use codeless_rpc::{
    ApproveReviewArgs, CommentReviewArgs, ListReviewsArgs, ListReviewsResult, StopReviewArgs,
};
use codeless_types::Review;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub async fn rpc_list_reviews(
    state: State<'_, AppState>,
    args: ListReviewsArgs,
) -> CommandResult<ListReviewsResult> {
    Ok(state.rpc.list_reviews(args).await?)
}

#[tauri::command]
pub async fn rpc_approve_review(
    state: State<'_, AppState>,
    args: ApproveReviewArgs,
) -> CommandResult<Review> {
    Ok(state.rpc.approve_review(args).await?)
}

#[tauri::command]
pub async fn rpc_comment_review(
    state: State<'_, AppState>,
    args: CommentReviewArgs,
) -> CommandResult<Review> {
    Ok(state.rpc.comment_review(args).await?)
}

#[tauri::command]
pub async fn rpc_stop_review(
    state: State<'_, AppState>,
    args: StopReviewArgs,
) -> CommandResult<Review> {
    Ok(state.rpc.stop_review(args).await?)
}
