//! End-to-end exercise of the review RPC surface. Drives the
//! `InProcessRpc` directly, since reviews are gates on stages and
//! don't depend on the runner/job-drive path — the SCOPE.md state
//! machine only cares about the transition guards.

use codeless_rpc::{
    AddRepoArgs, ApproveReviewArgs, CommentReviewArgs, ListReviewsArgs, RpcError, RpcServer,
    StopReviewArgs, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::{
    GitAuth, Review, ReviewId, ReviewStatus, Stage, StageId, StageStatus, UnixMillis,
};

/// Seeds the repo→job→stage→review chain so the FK constraints on
/// `reviews.stage_id` are satisfied. Returns the seeded review with
/// status `Pending`. Reviews do not own a FK to jobs directly — the
/// link is through the stage — but inserting a stage requires a job
/// and a job requires a repo, so the full chain has to be there.
async fn seed_pending(rpc: &InProcessRpc) -> Review {
    seed_pending_on_stage(rpc, new_stage(rpc).await).await
}

async fn seed_pending_on_stage(rpc: &InProcessRpc, stage_id: StageId) -> Review {
    let review = Review {
        id: ReviewId::new(),
        stage_id,
        status: ReviewStatus::Pending,
        comment: None,
        requested_at: UnixMillis(1_000),
        resolved_at: None,
    };
    rpc.store().insert_review(&review).await.unwrap();
    review
}

async fn new_stage(rpc: &InProcessRpc) -> StageId {
    let suffix = ReviewId::new();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: format!("demo-{suffix}"),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/codeless-demo-not-used".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-review-test".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    let stage = Stage {
        id: StageId::new(),
        job_id: job.id,
        ordinal: 0,
        name: "verify".into(),
        status: StageStatus::AwaitingReview,
        verify_cmd: None,
        started_at: None,
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
    };
    rpc.store().insert_stage(&stage).await.unwrap();
    stage.id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_marks_resolved_and_publishes_event() {
    let rpc = InProcessRpc::new().await.unwrap();
    let review = seed_pending(&rpc).await;

    let updated = rpc
        .approve_review(ApproveReviewArgs {
            review_id: review.id,
        })
        .await
        .unwrap();

    assert_eq!(updated.status, ReviewStatus::Approved);
    assert!(updated.resolved_at.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_rejects_already_resolved() {
    let rpc = InProcessRpc::new().await.unwrap();
    let review = seed_pending(&rpc).await;
    rpc.approve_review(ApproveReviewArgs {
        review_id: review.id,
    })
    .await
    .unwrap();

    let err = rpc
        .approve_review(ApproveReviewArgs {
            review_id: review.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Conflict(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_marks_resolved_and_blocks_approve() {
    let rpc = InProcessRpc::new().await.unwrap();
    let review = seed_pending(&rpc).await;

    let stopped = rpc
        .stop_review(StopReviewArgs {
            review_id: review.id,
        })
        .await
        .unwrap();
    assert_eq!(stopped.status, ReviewStatus::Stopped);

    let err = rpc
        .approve_review(ApproveReviewArgs {
            review_id: review.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Conflict(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comment_preserves_status_and_stacks_with_resolve() {
    let rpc = InProcessRpc::new().await.unwrap();
    let review = seed_pending(&rpc).await;

    let commented = rpc
        .comment_review(CommentReviewArgs {
            review_id: review.id,
            comment: "please double-check the migration".into(),
        })
        .await
        .unwrap();
    assert_eq!(commented.status, ReviewStatus::Pending);
    assert_eq!(
        commented.comment.as_deref(),
        Some("please double-check the migration")
    );

    rpc.approve_review(ApproveReviewArgs {
        review_id: review.id,
    })
    .await
    .unwrap();

    let reloaded = rpc.store().get_review(review.id).await.unwrap().unwrap();
    assert_eq!(reloaded.status, ReviewStatus::Approved);
    assert_eq!(
        reloaded.comment.as_deref(),
        Some("please double-check the migration"),
        "approve must not clobber an existing comment"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_filters_by_stage_and_status() {
    let rpc = InProcessRpc::new().await.unwrap();
    let stage_a = new_stage(&rpc).await;
    let stage_b = new_stage(&rpc).await;
    let r_a1 = seed_pending_on_stage(&rpc, stage_a).await;
    let _r_a2 = seed_pending_on_stage(&rpc, stage_a).await;
    let _r_b = seed_pending_on_stage(&rpc, stage_b).await;
    rpc.approve_review(ApproveReviewArgs { review_id: r_a1.id })
        .await
        .unwrap();

    let by_stage = rpc
        .list_reviews(ListReviewsArgs {
            job_id: None,
            stage_id: Some(stage_a),
            status: None,
        })
        .await
        .unwrap();
    assert_eq!(by_stage.reviews.len(), 2);

    let only_pending = rpc
        .list_reviews(ListReviewsArgs {
            job_id: None,
            stage_id: Some(stage_a),
            status: Some(ReviewStatus::Pending),
        })
        .await
        .unwrap();
    assert_eq!(only_pending.reviews.len(), 1);
    assert_eq!(only_pending.reviews[0].status, ReviewStatus::Pending);

    let all = rpc.list_reviews(ListReviewsArgs::default()).await.unwrap();
    assert_eq!(all.reviews.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_unknown_id_is_not_found() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .approve_review(ApproveReviewArgs {
            review_id: ReviewId::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}
