//! Cap-driven cancellation: `drive_job` watches `cost_cap_cents` and
//! `wall_clock_cap_ms` against the live rollup + a deadline timer,
//! and fires the runner's `CancellationToken` plus a
//! `JobStopped { reason }` event when either cap trips. The cost
//! path goes through `EventBus::publish` — which rolls cost into the
//! `jobs` row first — so a single `AiMessageComplete` is enough to
//! exceed the cap and trigger the stop.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{CostCents, Event, GitAuth, JobStatus, StopReason, TaskId};

async fn fresh_job(rpc: &InProcessRpc, cost_cap: i64, wall_clock_ms: i64) -> codeless_types::JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/unused".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-cap".into(),
        workspace_mode: None,
        cost_cap_cents: cost_cap,
        wall_clock_cap_ms: wall_clock_ms,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        start_immediately: true,
    })
    .await
    .unwrap()
    .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cost_cap_fires_job_stopped_with_cost_cap_reason() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_job(&rpc, 50, 60_000).await;

    let task_id = TaskId::new();
    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::AiMessageComplete {
            task_id,
            input_tokens: 100,
            output_tokens: 200,
            cost_cents: CostCents(100),
        }),
        // Park long enough that the watcher has a chance to react;
        // `MockRunner` does not honour `ctx.cancel`, so the stop path
        // we exercise here is the driver's terminal-row check after
        // `MockRunner` returns naturally.
        MockStep::Sleep(Duration::from_millis(200)),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    drive_job(&rpc, job_id, runner, None).await.unwrap();

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Stopped);
    assert_eq!(job.stop_reason, Some(StopReason::CostCap));
    assert!(
        job.cost_cents.0 >= 50,
        "cost rollup ran before the watcher fired: got {}",
        job.cost_cents.0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wall_clock_cap_fires_job_stopped_with_wall_clock_reason() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_job(&rpc, 0, 50).await;

    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Sleep(Duration::from_millis(400)),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    drive_job(&rpc, job_id, runner, None).await.unwrap();

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Stopped);
    assert_eq!(job.stop_reason, Some(StopReason::WallClock));
}

// Cap trip on a job whose stage already captured a runner session
// id is *resumable* — the watcher writes `Paused`, not `Stopped`,
// and publishes `JobPaused`. `resume_job` (A0) accepts the row
// and re-fires the stage with `--continue <session_id>`.
//
// This is the behaviour difference between a job that ran far
// enough to capture a session and one that didn't: only the
// former can be resumed without re-deriving the codebase from
// scratch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cost_cap_pauses_when_stage_has_captured_session() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_job(&rpc, 50, 60_000).await;

    // Plant a stage row with a captured session id BEFORE the
    // runner starts, so the watcher's `has_captured_session`
    // check sees it when the cost-cap fires.
    let stage = codeless_types::Stage {
        id: codeless_types::StageId::new(),
        job_id,
        ordinal: 0,
        name: "s".into(),
        status: codeless_types::StageStatus::Running,
        verify_cmd: None,
        started_at: Some(codeless_types::UnixMillis(0)),
        ended_at: None,
        session_id: Some("sess-captured".into()),
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
    };
    rpc.store().insert_stage(&stage).await.unwrap();

    let task_id = TaskId::new();
    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::AiMessageComplete {
            task_id,
            input_tokens: 100,
            output_tokens: 200,
            cost_cents: CostCents(100),
        }),
        MockStep::Sleep(Duration::from_millis(200)),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    drive_job(&rpc, job_id, runner, None).await.unwrap();

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .unwrap();
    assert_eq!(
        job.status,
        JobStatus::Paused,
        "cap on a stage with a captured session_id must pause, not stop"
    );
    assert_eq!(job.stop_reason, Some(StopReason::CostCap));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_caps_mean_unlimited_and_run_completes_normally() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_job(&rpc, 0, 0).await;

    let task_id = TaskId::new();
    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::AiMessageComplete {
            task_id,
            input_tokens: 10,
            output_tokens: 20,
            cost_cents: CostCents(9_999),
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    drive_job(&rpc, job_id, runner, None).await.unwrap();

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .unwrap();
    assert_eq!(
        job.status,
        JobStatus::Completed,
        "cap=0 must not interrupt a running job; got stop_reason={:?}",
        job.stop_reason
    );
}
