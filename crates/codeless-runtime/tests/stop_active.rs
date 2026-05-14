//! `stop_active` umbrella RPC: combines `stop_job` (when the row is
//! Running / AwaitingReview / Queued) with `cancel_chat_task` against
//! every in-flight chat turn whose `session_id` matches the job. The
//! UI's unified stop button calls this so it works for chat-over-
//! completed-jobs as well as the live-driver case.

use codeless_rpc::{AddRepoArgs, RpcServer, StopActiveArgs, SubmitJobArgs};
use codeless_runtime::{ChatCancelEntry, InProcessRpc};
use codeless_types::{GitAuth, Job, JobId, JobStatus, TaskId, UnixMillis};
use tokio_util::sync::CancellationToken;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_job(rpc: &InProcessRpc) -> Job {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: Some("mock".into()),
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("hi".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/stop-active".into(),
        workspace_mode: None,
        cost_cap_cents: 100,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        start_immediately: false,
    })
    .await
    .unwrap()
}

async fn force_status(rpc: &InProcessRpc, mut job: Job, status: JobStatus) -> Job {
    job.status = status;
    if matches!(status, JobStatus::Running) {
        job.started_at = Some(UnixMillis(0));
    }
    rpc.store().update_job(&job).await.unwrap();
    job
}

fn register_chat(rpc: &InProcessRpc, job_id: JobId) -> (TaskId, CancellationToken) {
    let task_id = TaskId::new();
    let token = CancellationToken::new();
    rpc.chat_cancels().lock().insert(
        task_id,
        ChatCancelEntry {
            job_id,
            token: token.clone(),
        },
    );
    (task_id, token)
}

#[tokio::test]
async fn stop_active_running_job_only() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc).await;
    let job = force_status(&rpc, job, JobStatus::Running).await;

    let result = rpc
        .stop_active(StopActiveArgs { job_id: job.id })
        .await
        .expect("stop_active");

    assert!(result.stopped_job, "job side should have fired");
    assert!(
        result.cancelled_chat_task_ids.is_empty(),
        "no chat turns were registered"
    );
    let after = rpc.store().get_job(job.id).await.unwrap().unwrap();
    assert_eq!(after.status, JobStatus::Stopped);
}

#[tokio::test]
async fn stop_active_chat_only_over_completed_job() {
    // The motivating bug: a `completed` job has no driver to stop,
    // but the user's chat turn against its worktree is still alive.
    // `stop_active` must fire the chat token without erroring on the
    // job side.
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc).await;
    let job = force_status(&rpc, job, JobStatus::Completed).await;
    let (task_id, token) = register_chat(&rpc, job.id);

    let result = rpc
        .stop_active(StopActiveArgs { job_id: job.id })
        .await
        .expect("stop_active");

    assert!(!result.stopped_job, "job is terminal — nothing to stop");
    assert_eq!(result.cancelled_chat_task_ids, vec![task_id]);
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn stop_active_both_running_and_chat() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc).await;
    let job = force_status(&rpc, job, JobStatus::Running).await;
    let (task_id, token) = register_chat(&rpc, job.id);

    let result = rpc
        .stop_active(StopActiveArgs { job_id: job.id })
        .await
        .expect("stop_active");

    assert!(result.stopped_job);
    assert_eq!(result.cancelled_chat_task_ids, vec![task_id]);
    assert!(token.is_cancelled());
    let after = rpc.store().get_job(job.id).await.unwrap().unwrap();
    assert_eq!(after.status, JobStatus::Stopped);
}

#[tokio::test]
async fn stop_active_neither_running_is_idempotent_ok() {
    // Terminal job, no chat turn registered — the umbrella is a
    // no-op success rather than a Conflict, so the UI can call it
    // unconditionally.
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc).await;
    let job = force_status(&rpc, job, JobStatus::Completed).await;

    let result = rpc
        .stop_active(StopActiveArgs { job_id: job.id })
        .await
        .expect("stop_active");

    assert!(!result.stopped_job);
    assert!(result.cancelled_chat_task_ids.is_empty());
}

#[tokio::test]
async fn stop_active_only_fires_chat_for_matching_job() {
    // A second job's chat turn must not be cancelled when stopping
    // this job — the registry's job_id key is the scoping rule.
    let rpc = InProcessRpc::new().await.unwrap();
    let target = seed_job(&rpc).await;
    let target = force_status(&rpc, target, JobStatus::Completed).await;
    let other_job_id = JobId::new();
    let (target_task, target_token) = register_chat(&rpc, target.id);
    let (_other_task, other_token) = register_chat(&rpc, other_job_id);

    let result = rpc
        .stop_active(StopActiveArgs { job_id: target.id })
        .await
        .unwrap();

    assert_eq!(result.cancelled_chat_task_ids, vec![target_task]);
    assert!(target_token.is_cancelled());
    assert!(!other_token.is_cancelled(), "unrelated chat untouched");
}
