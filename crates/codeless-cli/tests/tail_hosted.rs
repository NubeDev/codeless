//! End-to-end coverage of `codeless tail <job-id> --core URL --token T`.
//! Boots `codeless serve` over an ephemeral port against a temp DB,
//! seeds a completed job via the in-process runtime (mirroring
//! `tail_cli.rs`), then exercises the CLI in hosted-mode and asserts
//! the replayed JSON-line trace ends on `job-completed`.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use assert_cmd::Command as TestCommand;
use codeless_adapters_host::SecretStore;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{Event, GitAuth, JobId, TaskId, TaskStatus};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");
const TOKEN: &str = "tail-hosted-test-token";

async fn seed_completed_job(db: &Path) -> JobId {
    let rpc = InProcessRpc::with_file(db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/codeless-tail-hosted".into(),
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
            branch: "codeless/tail-hosted".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tail_replays_via_hosted_sse() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let job_id = seed_completed_job(&db).await;

    let mut server = Command::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let stderr = server.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line.clone()).is_err() {
                break;
            }
            if line.contains("listening on") {
                break;
            }
        }
    });

    let addr = (|| -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(idx) = line.find("http://") {
                    return line[idx + "http://".len()..].trim().to_string();
                }
            }
        }
        let _ = server.kill();
        panic!("server did not bind");
    })();

    let core_url = format!("http://{addr}");
    let out = TestCommand::new(BIN)
        .args([
            "--core",
            &core_url,
            "--token",
            TOKEN,
            "tail",
            &job_id.to_string(),
            "--timeout-secs",
            "10",
        ])
        .timeout(Duration::from_secs(15))
        .output()
        .expect("run tail");

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert!(
        out.status.success(),
        "tail failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"job-completed\""), "stdout: {stdout}");
    assert!(stdout.contains("\"task-started\""), "stdout: {stdout}");
}
