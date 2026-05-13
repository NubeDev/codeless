//! End-to-end exercise of `codeless tail <job-id>`. Seeds a real
//! repo + job, drives the job to `JobCompleted` via `MockRunner` so
//! the events table is fully populated, then invokes the CLI as a
//! subprocess against the same SQLite file. The subscribe path
//! replays persisted events (`since: None`), so tail sees every
//! envelope in order and exits on the terminal frame.

use std::sync::Arc;

use assert_cmd::Command as TestCommand;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{Event, GitAuth, JobId, TaskId, TaskStatus};
use predicates::str::contains;
use tempfile::TempDir;

async fn seed_completed_job(db_path: &std::path::Path) -> JobId {
    let rpc = InProcessRpc::with_file(db_path).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
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
            branch: "codeless/job-tail-test".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    let task = TaskId::new();
    let runner = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::TaskStarted { task_id: task }),
        MockStep::Emit(Event::TaskCompleted {
            task_id: task,
            status: TaskStatus::Completed,
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));
    drive_job(&rpc, job.id, runner, None).await.unwrap();
    job.id
}

#[test]
fn tail_replays_persisted_events_and_exits_on_completion() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let db_str = db.to_str().unwrap().to_string();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let job_id = rt.block_on(seed_completed_job(&db));

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "tail", &job_id.to_string()])
        .assert()
        .success()
        .stdout(contains("\"type\":\"job-started\""))
        .stdout(contains("\"type\":\"task-started\""))
        .stdout(contains("\"type\":\"task-completed\""))
        .stdout(contains("\"type\":\"job-completed\""));
}

#[test]
fn tail_rejects_invalid_job_id() {
    let dir = TempDir::new().unwrap();
    let db_str = dir.path().join("codeless.db").to_str().unwrap().to_string();

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "tail", "not-a-ulid"])
        .assert()
        .failure()
        .stderr(contains("invalid job id"));
}
