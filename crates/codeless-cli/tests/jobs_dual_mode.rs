//! `codeless jobs list/get/stop` against both local-mode (`--db`)
//! and hosted-mode (`--core` + `--token`). Mirrors the structure of
//! `repos_hosted_cli.rs`: pre-seed via `InProcessRpc`, then exec the
//! CLI as a subprocess for the verb under test.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use assert_cmd::Command as TestCommand;
use codeless_adapters_host::SecretStore;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{GitAuth, JobId};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");
const TOKEN: &str = "jobs-dual-mode-token";

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed(db: &Path) -> JobId {
    let rpc = InProcessRpc::with_file(db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "seeded".into(),
            clone_url: "https://example.test/seeded.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/seeded-jobs".into(),
            git_auth: token_auth(),
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
            branch: "feat/x".into(),
            workspace_mode: None,
            cost_cap_cents: 100,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    job.id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_list_local_mode() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let job_id = seed(&db).await;

    let out = TestCommand::new(BIN)
        .args(["--db", db.to_str().unwrap(), "jobs", "list"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&job_id.to_string()),
        "missing job id in {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_get_local_mode_prints_pretty_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let job_id = seed(&db).await;

    let out = TestCommand::new(BIN)
        .args([
            "--db",
            db.to_str().unwrap(),
            "jobs",
            "get",
            &job_id.to_string(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("{"));
    assert!(stdout.contains(&job_id.to_string()));
    assert!(stdout.contains("\"status\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_stop_local_mode_then_get_shows_stopped() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let job_id = seed(&db).await;

    TestCommand::new(BIN)
        .args([
            "--db",
            db.to_str().unwrap(),
            "jobs",
            "stop",
            &job_id.to_string(),
        ])
        .assert()
        .success();

    let out = TestCommand::new(BIN)
        .args([
            "--db",
            db.to_str().unwrap(),
            "jobs",
            "get",
            &job_id.to_string(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"status\": \"stopped\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"stop_reason\": \"user\""),
        "stdout: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_dual_mode_hosted_round_trip() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let job_id = seed(&db).await;

    // --no-driver: this test asserts on stop_job semantics against
    // a queued job. The background driver would complete the job
    // before the stop call lands; that path is exercised separately.
    let mut server = Command::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--no-driver",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

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
    let core = format!("http://{addr}");

    let out = TestCommand::new(BIN)
        .args(["--core", &core, "--token", TOKEN, "jobs", "list"])
        .timeout(Duration::from_secs(10))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(&job_id.to_string()));

    TestCommand::new(BIN)
        .args([
            "--core",
            &core,
            "--token",
            TOKEN,
            "jobs",
            "stop",
            &job_id.to_string(),
        ])
        .timeout(Duration::from_secs(10))
        .assert()
        .success();

    let out = TestCommand::new(BIN)
        .args([
            "--core",
            &core,
            "--token",
            TOKEN,
            "jobs",
            "get",
            &job_id.to_string(),
        ])
        .timeout(Duration::from_secs(10))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"status\": \"stopped\""),
        "stdout: {stdout}"
    );

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();
}
