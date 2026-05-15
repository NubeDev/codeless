//! Drives a queued job to terminal state through the `MockRunner`
//! harness. Asserts the framing events (`job-started` and one of
//! `job-completed` / `job-failed`) are emitted by the driver — never
//! by the runner — and that mid-run events the runner publishes pass
//! through the bus untouched. Also pins the state-machine guard:
//! `drive_job` on a non-`Queued` job is a `Conflict`.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, RpcError, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{Event, GitAuth, JobStatus, StageId, TaskId, TaskStatus};
use futures_util::StreamExt;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn wait_for<F>(stream: &mut codeless_rpc::EventStream, mut pred: F) -> Event
where
    F: FnMut(&Event) -> bool,
{
    let fut = async {
        loop {
            let item = stream.next().await.expect("stream ended");
            let env = item.expect("stream error");
            if pred(&env.event) {
                return env.event;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), fut)
        .await
        .expect("timed out waiting for event")
}

async fn fresh_queued_job(rpc: &InProcessRpc) -> codeless_types::JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .expect("add_repo");
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("anything".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-1".into(),
        workspace_mode: None,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        start_immediately: true,
    })
    .await
    .expect("submit_job")
    .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_job_to_completion_emits_started_then_runner_events_then_completed() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::TaskStarted {
            task_id: TaskId::new(),
        }),
        MockStep::Emit(Event::TaskCompleted {
            task_id: TaskId::new(),
            status: TaskStatus::Completed,
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    drive_job(&rpc, job_id, runner, None)
        .await
        .expect("drive_job");

    let ev = wait_for(&mut stream, |e| matches!(e, Event::JobStarted { .. })).await;
    assert!(matches!(ev, Event::JobStarted { job_id: j } if j == job_id));
    let ev = wait_for(&mut stream, |e| matches!(e, Event::TaskStarted { .. })).await;
    assert!(matches!(ev, Event::TaskStarted { .. }));
    let ev = wait_for(&mut stream, |e| matches!(e, Event::TaskCompleted { .. })).await;
    assert!(matches!(ev, Event::TaskCompleted { .. }));
    let ev = wait_for(&mut stream, |e| matches!(e, Event::JobCompleted { .. })).await;
    assert!(matches!(ev, Event::JobCompleted { job_id: j } if j == job_id));

    let final_job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(final_job.status, JobStatus::Completed);
    assert!(final_job.started_at.is_some());
    assert!(final_job.ended_at.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_job_failure_outcome_lands_as_failed() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::StageStarted {
            stage_id: StageId::new(),
            job_id,
            ordinal: 0,
            name: "scripted stage".into(),
        }),
        MockStep::Finish(RunnerOutcome::Failed {
            reason: "scripted failure".into(),
        }),
    ]));

    drive_job(&rpc, job_id, runner, None)
        .await
        .expect("drive_job");

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(job.status, JobStatus::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_job_refuses_already_terminal_job() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let runner_a = Arc::new(MockRunner::new(vec![MockStep::Finish(
        RunnerOutcome::Completed,
    )]));
    drive_job(&rpc, job_id, runner_a, None)
        .await
        .expect("first run");

    let runner_b = Arc::new(MockRunner::new(vec![MockStep::Finish(
        RunnerOutcome::Completed,
    )]));
    let err = drive_job(&rpc, job_id, runner_b, None).await.unwrap_err();
    assert!(matches!(err, RpcError::Conflict(_)), "{err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_during_run_wins_against_completion() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Sleep(Duration::from_millis(50)),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));

    let rpc_ref: &'static InProcessRpc = Box::leak(Box::new(rpc));
    let runner_clone = Arc::clone(&runner);
    let drive_handle =
        tokio::spawn(async move { drive_job(rpc_ref, job_id, runner_clone, None).await });

    tokio::time::sleep(Duration::from_millis(10)).await;
    rpc_ref
        .stop_job(codeless_rpc::StopJobArgs { job_id })
        .await
        .expect("stop_job");

    drive_handle.await.expect("join").expect("drive_job");

    let job = rpc_ref
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(job.status, JobStatus::Stopped);
}
