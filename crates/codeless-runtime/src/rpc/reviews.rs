use codeless_rpc::{
    ApproveReviewArgs, CommentReviewArgs, ListReviewsArgs, ListReviewsResult, RpcError, RpcResult,
    StopReviewArgs,
};
use codeless_types::{Event, Review, ReviewStatus};

use super::InProcessRpc;
use crate::time::now_ms;

/// Shared resolve-to-terminal helper for `approve_review` and `stop_review`.
/// Centralises the conflict/not-found checks so neither RPC can drift on
/// which transitions it accepts. The caller publishes the corresponding
/// event so the event-name choice stays at the call site.
async fn resolve_pending_review(
    rpc: &InProcessRpc,
    review_id: codeless_types::ReviewId,
    next: ReviewStatus,
) -> RpcResult<Review> {
    let mut review = rpc
        .store
        .get_review(review_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("review {review_id}")))?;
    if review.status != ReviewStatus::Pending {
        return Err(RpcError::Conflict(format!(
            "review {review_id} is already resolved ({:?})",
            review.status
        )));
    }
    let now = now_ms();
    review.status = next;
    review.resolved_at = Some(now);
    rpc.store.update_review(&review).await.map_err(super::db_err)?;
    Ok(review)
}

pub(super) async fn list_reviews(
    rpc: &InProcessRpc,
    args: ListReviewsArgs,
) -> RpcResult<ListReviewsResult> {
    Ok(ListReviewsResult {
        reviews: rpc
            .store
            .list_reviews(args.job_id, args.stage_id, args.status)
            .await
            .map_err(super::db_err)?,
    })
}

pub(super) async fn approve_review(
    rpc: &InProcessRpc,
    args: ApproveReviewArgs,
) -> RpcResult<Review> {
    let review = resolve_pending_review(rpc, args.review_id, ReviewStatus::Approved).await?;
    rpc.bus
        .publish(
            None,
            Some(review.stage_id),
            None,
            Event::ReviewApproved {
                review_id: review.id,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(review)
}

pub(super) async fn comment_review(
    rpc: &InProcessRpc,
    args: CommentReviewArgs,
) -> RpcResult<Review> {
    let mut review = rpc
        .store
        .get_review(args.review_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("review {}", args.review_id)))?;
    review.comment = Some(args.comment.clone());
    rpc.store.update_review(&review).await.map_err(super::db_err)?;
    rpc.bus
        .publish(
            None,
            Some(review.stage_id),
            None,
            Event::ReviewCommented {
                review_id: review.id,
                comment: args.comment,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(review)
}

pub(super) async fn stop_review(rpc: &InProcessRpc, args: StopReviewArgs) -> RpcResult<Review> {
    let review = resolve_pending_review(rpc, args.review_id, ReviewStatus::Stopped).await?;
    rpc.bus
        .publish(
            None,
            Some(review.stage_id),
            None,
            Event::ReviewStopped {
                review_id: review.id,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(review)
}
