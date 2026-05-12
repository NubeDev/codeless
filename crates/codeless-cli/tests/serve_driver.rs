//! `codeless serve` with the background driver enabled (the
//! default). A submitted mock-runner job must reach `Completed`
//! without anyone calling `codeless run` — that's the whole point
//! of the driver loop.
//!
//! The test:
//!   1. Boot the server with `--db <tmp>` and the driver default-on.
//!   2. Submit a `mock` job via the HTTP RPC surface.
//!   3. Subscribe SSE and wait for `job-completed`.
//!   4. Hit `/rpc/get_job` and assert `status == "completed"`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use codeless_adapters_host::SecretStore;
use codeless_rpc::{AddRepoArgs, RpcServer};
use codeless_runtime::InProcessRpc;
use codeless_types::{GitAuth, RepoId};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");
const TOKEN: &str = "driver-test-token";

async fn seed_repo(db: &std::path::Path) -> RepoId {
    let rpc = InProcessRpc::with_file(db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "driver-demo".into(),
            clone_url: "https://example.test/driver.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/driver-demo".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    repo.id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submitted_mock_job_reaches_completed_via_driver() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let repo_id = seed_repo(&db).await;

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
        .expect("spawn");

    let stderr = server.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    let addr = (|| -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(idx) = line.find("listening on http://") {
                    return line[idx + "listening on http://".len()..]
                        .trim()
                        .to_string();
                }
            }
        }
        let _ = server.kill();
        panic!("server did not bind");
    })();

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let submit: serde_json::Value = client
        .post(format!("{base}/rpc/submit_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "repo_id": repo_id.to_string(),
            "prompt": "hello",
            "template_yaml": null,
            "runner": "mock",
            "branch": "feat/driven",
            "cost_cap_cents": 0,
            "wall_clock_cap_ms": 60000,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job_id = submit["id"].as_str().expect("id").to_string();

    // Poll get_job until it goes terminal. The driver runs the
    // mock script `Finish(Completed)` immediately on a JobQueued
    // event, so a few hundred ms is plenty.
    let observed = poll_until_terminal(&client, &base, &job_id, Duration::from_secs(5)).await;

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert_eq!(
        observed["status"], "completed",
        "job did not complete: {observed}"
    );
    assert!(
        observed["ended_at"].is_number(),
        "ended_at missing: {observed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_runner_leaves_job_queued() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let repo_id = seed_repo(&db).await;

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
        .expect("spawn");

    let stderr = server.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let addr = (|| -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(idx) = line.find("listening on http://") {
                    return line[idx + "listening on http://".len()..]
                        .trim()
                        .to_string();
                }
            }
        }
        let _ = server.kill();
        panic!("server did not bind");
    })();

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let submit: serde_json::Value = client
        .post(format!("{base}/rpc/submit_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "repo_id": repo_id.to_string(),
            "prompt": "hi",
            "template_yaml": null,
            "runner": "unsupported-fake-runner",
            "branch": "x",
            "cost_cap_cents": 0,
            "wall_clock_cap_ms": 0,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job_id = submit["id"].as_str().unwrap().to_string();

    // 500ms is enough for the driver to refuse and skip; we expect
    // the job to stay Queued forever.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let fetched: serde_json::Value = client
        .post(format!("{base}/rpc/get_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "job_id": job_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert_eq!(
        fetched["status"], "queued",
        "job should remain queued: {fetched}"
    );
}

async fn poll_until_terminal(
    client: &reqwest::Client,
    base: &str,
    job_id: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let resp: serde_json::Value = client
            .post(format!("{base}/rpc/get_job"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "job_id": job_id }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let status = resp["status"].as_str().unwrap_or("");
        if matches!(status, "completed" | "failed" | "stopped") {
            return resp;
        }
        if std::time::Instant::now() >= deadline {
            return resp;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_runner_disabled_by_default_leaves_job_queued() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let repo_id = seed_repo(&db).await;

    // Server boots with default flags (anthropic disabled).
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
        .expect("spawn");

    let stderr = server.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let addr = (|| -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(idx) = line.find("listening on http://") {
                    return line[idx + "listening on http://".len()..]
                        .trim()
                        .to_string();
                }
            }
        }
        let _ = server.kill();
        panic!("server did not bind");
    })();

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let submit: serde_json::Value = client
        .post(format!("{base}/rpc/submit_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "repo_id": repo_id.to_string(),
            "prompt": "do a thing",
            "template_yaml": null,
            "runner": "anthropic",
            "branch": "x",
            "cost_cap_cents": 0,
            "wall_clock_cap_ms": 0,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job_id = submit["id"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_millis(500)).await;
    let fetched: serde_json::Value = client
        .post(format!("{base}/rpc/get_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "job_id": job_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert_eq!(fetched["status"], "queued", "{fetched}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn driver_provisions_worktree_when_root_set() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");
    let wt_root = dir.path().join("worktrees");
    let repo_path = dir.path().join("source-repo");

    // Bootstrap a real git repo so `git worktree add` has somewhere
    // to branch from. MockRunner ignores the worktree path itself,
    // but the driver's provision call still hits the real git
    // binary, which means we need a checkout that resolves.
    std::fs::create_dir_all(&repo_path).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&repo_path)
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "demo@example.com"]);
    git(&["config", "user.name", "Demo"]);
    std::fs::write(repo_path.join("README.md"), b"hi\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    // Register the repo against the real on-disk checkout.
    let rpc = InProcessRpc::with_file(&db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "wt-demo".into(),
            clone_url: format!("file://{}", repo_path.display()),
            default_branch: "master".into(),
            local_path: repo_path.display().to_string(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    drop(rpc);

    let mut server = Command::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--worktree-root",
            wt_root.to_str().unwrap(),
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");

    let stderr = server.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let addr = (|| -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(500)) {
                if let Some(idx) = line.find("listening on http://") {
                    return line[idx + "listening on http://".len()..]
                        .trim()
                        .to_string();
                }
            }
        }
        let _ = server.kill();
        panic!("server did not bind");
    })();

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let submit: serde_json::Value = client
        .post(format!("{base}/rpc/submit_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "repo_id": repo.id.to_string(),
            "prompt": "hi",
            "template_yaml": null,
            "runner": "mock",
            "branch": "ignored",
            "cost_cap_cents": 0,
            "wall_clock_cap_ms": 60000,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job_id = submit["id"].as_str().unwrap().to_string();

    let observed = poll_until_terminal(&client, &base, &job_id, Duration::from_secs(5)).await;

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert_eq!(
        observed["status"], "completed",
        "job did not complete: {observed}"
    );
    // drive_job removes the worktree on terminal status; we assert
    // that the branch was created (it survives `worktree remove`)
    // rather than that the directory still exists.
    let branches = std::process::Command::new("git")
        .current_dir(&repo_path)
        .args(["branch", "--list"])
        .output()
        .unwrap();
    let branches_out = String::from_utf8_lossy(&branches.stdout);
    assert!(
        branches_out.contains(&format!("codeless/job-{job_id}")),
        "expected job branch in: {branches_out}"
    );
}

// Keep `Arc<InProcessRpc>` import in scope for future expansion of
// the test suite without churn on the import list.
#[allow(dead_code)]
fn _imports_used() {
    let _: Option<Arc<InProcessRpc>> = None;
}
