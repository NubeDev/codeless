//! End-to-end coverage of `codeless --core URL --token T repos list`.
//! Boots the real `codeless` binary in `serve` mode, seeds a repo
//! via the local-mode CLI against the same DB, then exercises the
//! `repos list` verb in hosted-mode and asserts the seeded repo
//! appears in stdout.
//!
//! This is the smoke test for the whole hosted-mode round trip:
//! `HttpRpcClient` → reqwest → `codeless-server` → `InProcessRpc`
//! → SQLite. If any layer drifts, this test catches it before the
//! UI does.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use assert_cmd::Command as TestCommand;
use codeless_adapters_host::SecretStore;
use codeless_rpc::{AddRepoArgs, RpcServer};
use codeless_runtime::InProcessRpc;
use codeless_types::GitAuth;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repos_list_against_hosted_core() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let token = "hosted-cli-test-token";
    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", token).unwrap();
    store.save().unwrap();

    // Pre-seed the DB directly so the hosted CLI sees a populated
    // repo list — drives the round trip rather than the seeded
    // path itself.
    let rpc = InProcessRpc::with_file(&db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "seeded".into(),
            clone_url: "https://example.test/seeded.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/seeded".into(),
            git_auth: token_auth(),
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
        panic!("server did not announce bound address");
    })();

    let core_url = format!("http://{addr}");
    let out = TestCommand::new(BIN)
        .args(["--core", &core_url, "--token", token, "repos", "list"])
        .timeout(Duration::from_secs(10))
        .output()
        .expect("run repos list");

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert!(
        out.status.success(),
        "repos list failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&repo.id.to_string()),
        "missing repo id in: {stdout}"
    );
    assert!(stdout.contains("seeded"), "missing repo name in: {stdout}");
}

#[test]
fn repos_list_local_mode_works_without_core() {
    // Local-mode `repos list` against a fresh `:memory:` DB returns
    // "(no repos)" on stderr and exits 0 — the no-network path the
    // dual-mode dispatch keeps alive.
    TestCommand::new(BIN)
        .args(["repos", "list"])
        .assert()
        .success();
}

#[test]
fn token_without_core_rejected() {
    let out = TestCommand::new(BIN)
        .args(["--token", "anything", "repos", "list"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--token only meaningful with --core"),
        "got: {stderr}"
    );
}
