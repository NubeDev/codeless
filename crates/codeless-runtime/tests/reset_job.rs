//! `reset_job` is the user-driven recovery hatch for stuck jobs: a
//! `Queued` row whose driver kept failing past its retry budget, plus
//! the terminal `Failed` / `Stopped` rows the operator wants to re-edit
//! without paying the `resume_job` cap-bump dance. Each accepted source
//! status collapses back to `Draft` with `worktree_path`, `stop_reason`,
//! and `ended_at` cleared; `Running` (and the other non-stuck states)
//! is refused with `Conflict`.

use codeless_rpc::{AddRepoArgs, GetJobArgs, ResetJobArgs, RpcError, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{Event, GitAuth, JobStatus, StopReason, UnixMillis};

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_job(rpc: &InProcessRpc, status: JobStatus) -> codeless_types::Job {
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
    let mut job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("stuck".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/reset-me".into(),
            workspace_mode: None,
            cost_cap_cents: 500,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    job.status = status;
    job.stop_reason = match status {
        JobStatus::Failed => Some(StopReason::RunnerCrash),
        JobStatus::Stopped => Some(StopReason::User),
        _ => None,
    };
    job.ended_at = match status {
        JobStatus::Failed | JobStatus::Stopped => Some(UnixMillis(123)),
        _ => None,
    };
    job.worktree_path = Some("/tmp/demo/.codeless-worktrees/job-x".into());
    rpc.store().update_job(&job).await.unwrap();
    job
}

#[tokio::test]
async fn reset_job_returns_queued_to_draft() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc, JobStatus::Queued).await;

    let after = rpc
        .reset_job(ResetJobArgs { job_id: job.id })
        .await
        .unwrap();

    assert_eq!(after.status, JobStatus::Draft);
    assert_eq!(after.stop_reason, None);
    assert_eq!(after.ended_at, None);
    assert_eq!(after.worktree_path, None);

    let reread = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert_eq!(reread.status, JobStatus::Draft);
    assert_eq!(reread.worktree_path, None);
}

#[tokio::test]
async fn reset_job_returns_failed_to_draft() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc, JobStatus::Failed).await;

    let after = rpc
        .reset_job(ResetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(after.status, JobStatus::Draft);
    assert_eq!(after.stop_reason, None);
    assert_eq!(after.ended_at, None);
    assert_eq!(after.worktree_path, None);
}

#[tokio::test]
async fn reset_job_returns_stopped_to_draft() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc, JobStatus::Stopped).await;

    let after = rpc
        .reset_job(ResetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(after.status, JobStatus::Draft);
    assert_eq!(after.stop_reason, None);
    assert_eq!(after.ended_at, None);
    assert_eq!(after.worktree_path, None);
}

#[tokio::test]
async fn reset_job_refuses_running() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc, JobStatus::Running).await;

    let err = rpc
        .reset_job(ResetJobArgs { job_id: job.id })
        .await
        .unwrap_err();
    assert!(
        matches!(err, RpcError::Conflict(_)),
        "expected Conflict for Running, got {err:?}"
    );
    let reread = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert_eq!(
        reread.status,
        JobStatus::Running,
        "refused reset must leave the row untouched"
    );
}

#[tokio::test]
async fn reset_job_publishes_job_reset_event() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_job(&rpc, JobStatus::Failed).await;

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            Some(codeless_types::EventCursor(0)),
        )
        .await
        .unwrap();

    rpc.reset_job(ResetJobArgs { job_id: job.id })
        .await
        .unwrap();

    use tokio_stream::StreamExt;
    let mut saw_reset = false;
    for _ in 0..32 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
        match item {
            Ok(Some(Ok(env))) => {
                if let Event::JobReset {
                    job_id: jid,
                    previous_status,
                } = env.event
                {
                    assert_eq!(jid, job.id);
                    assert_eq!(previous_status, JobStatus::Failed);
                    saw_reset = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_reset, "JobReset event must fire on reset");
}
