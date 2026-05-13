//! End-to-end coverage of `codeless serve` with the
//! `WebhookNotifier` wired in. Boots a `wiremock` server as the
//! webhook receiver, configures the secrets file with the URL +
//! HMAC key, and submits a mock job whose prompt is the `FAIL`
//! sentinel — the driver runs it, the runtime emits `JobFailed`,
//! and the notifier POSTs an HMAC-signed body to wiremock.
//!
//! Partial-config behaviour (one half of the keys present) is also
//! tested: the server must refuse to boot rather than silently skip
//! the notifier.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use assert_cmd::Command as TestCommand;
use codeless_adapters_host::SecretStore;
use codeless_rpc::{AddRepoArgs, RpcServer};
use codeless_runtime::InProcessRpc;
use codeless_types::{GitAuth, RepoId};
use tempfile::TempDir;
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BIN: &str = env!("CARGO_BIN_EXE_codeless");
const TOKEN: &str = "notifier-test-token";
// Hex-encoded 16-byte HMAC key (32 hex chars). Constant for the
// test so the assert-side can re-derive the signature if it ever
// needs to; today we only assert that the header is present.
const HMAC_KEY_HEX: &str = "deadbeefdeadbeefdeadbeefdeadbeef";

async fn seed_repo(db: &std::path::Path) -> RepoId {
    let rpc = InProcessRpc::with_file(db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "notif-demo".into(),
            clone_url: "https://example.test/notif.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/notif-demo".into(),
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
async fn job_failed_event_fires_signed_webhook() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let wiremock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codeless-webhook"))
        .and(header_exists("x-codeless-signature"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&wiremock_server)
        .await;
    let webhook_url = format!("{}/codeless-webhook", wiremock_server.uri());

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.set("notifier_webhook_url", &webhook_url).unwrap();
    store
        .set("notifier_webhook_hmac_key_hex", HMAC_KEY_HEX)
        .unwrap();
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
    client
        .post(format!("{base}/rpc/submit_job"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "repo_id": repo_id.to_string(),
            "prompt": "FAIL",
            "template_yaml": null,
            "runner": "mock",
            "branch": "x",
            "cost_cap_cents": 0,
            "wall_clock_cap_ms": 60000,
            "start_immediately": true,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    // wiremock's Mock::expect(1) is asserted on Drop. To force it
    // here while we still hold the handle, give the driver + notifier
    // a beat to fire and then drop the server.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    let received = wiremock_server.received_requests().await.unwrap();
    assert!(
        received.iter().any(|r| r.url.path() == "/codeless-webhook"
            && r.headers.get("x-codeless-signature").is_some()),
        "no signed webhook POST observed: {received:?}"
    );
    // Body should mention the job-failed event tag.
    let body_str = String::from_utf8_lossy(&received[0].body);
    assert!(
        body_str.contains("job-failed"),
        "webhook body did not contain job-failed: {body_str}",
    );
}

#[test]
fn partial_webhook_config_refuses_to_boot() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    // Only the URL, no HMAC key — the operator's typo. Server
    // refuses to start rather than silently skipping the notifier.
    {
        let mut store = SecretStore::open(&secrets).unwrap();
        store.set("core_bearer_token", "any").unwrap();
        store
            .set("notifier_webhook_url", "http://example.test")
            .unwrap();
        store.save().unwrap();
    }

    let out = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
        ])
        .timeout(Duration::from_secs(5))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("partial webhook config"),
        "expected partial-config error, got: {stderr}"
    );
}
