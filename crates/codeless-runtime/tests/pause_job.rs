//! `pause_job` moves a Running (or AwaitingReview) job to Paused
//! with `stop_reason = User` and publishes `Event::JobPaused`. The
//! row's branch / worktree / captured `Stage.session_id` are
//! preserved; the cap-watcher's bus subscription fires the in-flight
//! runner's cancellation token. `resume_job` accepts the resulting
//! row and re-fires the stage with `--continue`.

use codeless_rpc::{
    AddRepoArgs, GetJobArgs, PauseJobArgs, ResumeJobArgs, RpcServer, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::{Event, GitAuth, JobStatus, StopReason};

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_running_job(rpc: &InProcessRpc) -> codeless_types::Job {
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
            prompt: Some("work in progress".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/pause-me".into(),
            workspace_mode: None,
            cost_cap_cents: 500,
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
    // submit lands in Queued; force Running so the test exercises
    // the Running -> Paused transition that pause_job is for.
    job.status = JobStatus::Running;
    job.started_at = Some(codeless_types::UnixMillis(0));
    rpc.store().update_job(&job).await.unwrap();
    job
}

#[tokio::test]
async fn pause_job_moves_running_to_paused_with_user_reason() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_running_job(&rpc).await;
    let original_branch = job.branch.clone();
    let original_cost_cap = job.cost_cap_cents.0;

    rpc.pause_job(PauseJobArgs { job_id: job.id })
        .await
        .unwrap();

    let after = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert_eq!(after.status, JobStatus::Paused);
    assert_eq!(after.stop_reason, Some(StopReason::User));
    assert!(
        after.ended_at.is_some(),
        "paused row should record ended_at"
    );
    assert_eq!(after.branch, original_branch, "branch preserved");
    assert_eq!(
        after.cost_cap_cents.0, original_cost_cap,
        "caps unchanged on pause"
    );
}

#[tokio::test]
async fn pause_job_publishes_job_paused_event() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_running_job(&rpc).await;

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            Some(codeless_types::EventCursor(0)),
        )
        .await
        .unwrap();

    rpc.pause_job(PauseJobArgs { job_id: job.id })
        .await
        .unwrap();

    use tokio_stream::StreamExt;
    let mut saw_paused = false;
    for _ in 0..32 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
        match item {
            Ok(Some(Ok(env))) => {
                if let Event::JobPaused {
                    job_id: jid,
                    reason,
                } = env.event
                {
                    assert_eq!(jid, job.id);
                    assert_eq!(reason, StopReason::User);
                    saw_paused = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_paused, "JobPaused event must fire on pause");
}

#[tokio::test]
async fn pause_job_rejects_non_pausable_states() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_running_job(&rpc).await;
    // Push back to Draft to force the conflict path.
    job.status = JobStatus::Draft;
    job.started_at = None;
    rpc.store().update_job(&job).await.unwrap();

    let err = rpc
        .pause_job(PauseJobArgs { job_id: job.id })
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("running") || msg.to_lowercase().contains("pausable"),
        "expected a Running/AwaitingReview conflict, got: {msg}",
    );
}

#[tokio::test]
async fn paused_job_is_resumable() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_running_job(&rpc).await;

    rpc.pause_job(PauseJobArgs { job_id: job.id })
        .await
        .unwrap();
    let resumed = rpc
        .resume_job(ResumeJobArgs {
            job_id: job.id,
            additional_cost_cap_cents: None,
            additional_wall_clock_cap_ms: None,
            bypass_failing_stage: false,
        })
        .await
        .unwrap();
    assert_eq!(
        resumed.status,
        JobStatus::Queued,
        "resume_job must re-queue a paused row"
    );
}
