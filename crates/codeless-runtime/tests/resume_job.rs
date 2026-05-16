//! `resume_job` re-queues a `Stopped` or `Failed` job in place — same
//! row, same branch, same worktree, same captured per-stage
//! `Stage.session_id`. The next claude task on that stage passes
//! `--continue <session_id>` and the agent picks up where it left off
//! (SCOPE.md hard rule #1: the stage is the session boundary; within
//! a stage the runner session is continuous and a cost-cap is a
//! pause, not a reset). The companion code path that fills
//! `Stage.session_id` lives in `StageRecorder` — these tests only
//! cover the resume RPC's contract.

use codeless_rpc::{AddRepoArgs, ResumeJobArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{Event, GitAuth, JobStatus, StopReason};

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_stopped_job(rpc: &InProcessRpc, reason: StopReason) -> codeless_types::Job {
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
            prompt: Some("do work".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/resume-me".into(),
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
    job.status = JobStatus::Stopped;
    job.stop_reason = Some(reason);
    job.ended_at = Some(codeless_types::UnixMillis(123_456));
    rpc.store().update_job(&job).await.unwrap();
    job
}

#[tokio::test]
async fn resume_job_requeues_stopped_in_place() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_stopped_job(&rpc, StopReason::CostCap).await;
    let original_branch = job.branch.clone();
    let original_cost_cap = job.cost_cap_cents.0;

    let resumed = rpc
        .resume_job(ResumeJobArgs {
            job_id: job.id,
            additional_cost_cap_cents: None,
            additional_wall_clock_cap_ms: None,
            bypass: false,
            next_stage_comment: None,
        })
        .await
        .unwrap();

    assert_eq!(resumed.id, job.id, "resume mutates the existing row");
    assert_eq!(resumed.status, JobStatus::Queued);
    assert_eq!(resumed.stop_reason, None, "live row clears the reason");
    assert_eq!(resumed.ended_at, None);
    assert_eq!(resumed.branch, original_branch, "branch preserved");
    assert_eq!(
        resumed.cost_cap_cents.0, original_cost_cap,
        "caps unchanged when no bump requested"
    );
}

#[tokio::test]
async fn resume_job_applies_cap_bumps_additively() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_stopped_job(&rpc, StopReason::CostCap).await;

    let resumed = rpc
        .resume_job(ResumeJobArgs {
            job_id: job.id,
            additional_cost_cap_cents: Some(1500),
            additional_wall_clock_cap_ms: Some(30_000),
            bypass: false,
            next_stage_comment: None,
        })
        .await
        .unwrap();

    assert_eq!(
        resumed.cost_cap_cents.0,
        job.cost_cap_cents.0 + 1500,
        "cost cap is additive"
    );
    assert_eq!(
        resumed.wall_clock_cap_ms,
        job.wall_clock_cap_ms + 30_000,
        "wall-clock cap is additive"
    );
}

#[tokio::test]
async fn resume_job_emits_job_resumed_with_previous_reason() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_stopped_job(&rpc, StopReason::CostCap).await;

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            Some(codeless_types::EventCursor(0)),
        )
        .await
        .unwrap();

    rpc.resume_job(ResumeJobArgs {
        job_id: job.id,
        additional_cost_cap_cents: None,
        additional_wall_clock_cap_ms: None,
        bypass: false,
        next_stage_comment: None,
    })
    .await
    .unwrap();

    use tokio_stream::StreamExt;
    let mut saw_resumed = false;
    for _ in 0..32 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
        match item {
            Ok(Some(Ok(env))) => {
                if let Event::JobResumed {
                    job_id,
                    previous_reason,
                    actor,
                } = env.event
                {
                    assert_eq!(job_id, job.id);
                    assert_eq!(previous_reason, Some(StopReason::CostCap));
                    // The base RPC has no surface context, so the
                    // emitted event carries `None`. A future Slack /
                    // assistant wrapper will set this; pinning the
                    // default here protects the wire-default contract.
                    assert!(
                        actor.is_none(),
                        "base resume_job must emit actor=None; got {actor:?}",
                    );
                    saw_resumed = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_resumed, "JobResumed event must fire on resume");
}

#[tokio::test]
async fn resume_job_rejects_non_terminal_states() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_stopped_job(&rpc, StopReason::User).await;
    // Push the job back to a live state to force the conflict path.
    job.status = JobStatus::Running;
    job.stop_reason = None;
    job.ended_at = None;
    rpc.store().update_job(&job).await.unwrap();

    let err = rpc
        .resume_job(ResumeJobArgs {
            job_id: job.id,
            additional_cost_cap_cents: None,
            additional_wall_clock_cap_ms: None,
            bypass: false,
            next_stage_comment: None,
        })
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("resumable") || msg.contains("running"),
        "expected resumable-state conflict, got: {msg}",
    );
}

#[tokio::test]
async fn resume_job_works_on_failed_jobs_too() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_stopped_job(&rpc, StopReason::RunnerCrash).await;
    job.status = JobStatus::Failed;
    rpc.store().update_job(&job).await.unwrap();

    let resumed = rpc
        .resume_job(ResumeJobArgs {
            job_id: job.id,
            additional_cost_cap_cents: None,
            additional_wall_clock_cap_ms: None,
            bypass: false,
            next_stage_comment: None,
        })
        .await
        .unwrap();
    assert_eq!(resumed.status, JobStatus::Queued);
}
