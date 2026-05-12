//! End-to-end coverage of the `codeless serve` verb.
//!
//! `--init-token` is exercised purely through `assert_cmd`: the
//! generated token must land in the secrets file and round-trip
//! through `SecretStore::open`.
//!
//! The bearer-gate test boots the actual binary against a temp DB
//! and an ephemeral port (`127.0.0.1:0`), parses the bound address
//! out of the server's stderr "listening on" line, and exercises
//! `/rpc/list_repos` with and without the configured token. This
//! mirrors what the browser will do.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use assert_cmd::Command as TestCommand;
use codeless_adapters_host::SecretStore;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");

#[test]
fn init_token_writes_random_hex_and_prints_it_once() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    let out = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--init-token",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let token = stdout.trim();
    assert_eq!(token.len(), 32, "expected 32 hex chars, got {token:?}");
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "non-hex char in {token:?}"
    );

    let store = SecretStore::open(&secrets).unwrap();
    assert_eq!(store.get("core_bearer_token"), Some(token));
}

#[test]
fn init_token_refuses_to_overwrite_without_force() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--init-token",
        ])
        .assert()
        .success();

    let out = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--init-token",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already configured"), "got: {stderr}");
}

#[test]
fn init_token_force_rotates() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    let first = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--init-token",
        ])
        .output()
        .unwrap();
    let first_token = String::from_utf8(first.stdout).unwrap().trim().to_string();

    let second = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--init-token",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_token = String::from_utf8(second.stdout).unwrap().trim().to_string();

    assert_ne!(first_token, second_token);
    let store = SecretStore::open(&secrets).unwrap();
    assert_eq!(store.get("core_bearer_token"), Some(second_token.as_str()));
}

#[test]
fn serve_route_requires_bearer_token() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let token = "integration-test-token-abcdef";
    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", token).unwrap();
    store.save().unwrap();

    let mut child = Command::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--require-token",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn codeless serve");

    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let reader_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line.clone()).is_err() {
                break;
            }
            if line.contains("listening on http://") {
                break;
            }
        }
    });

    let mut bound = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(line) => {
                if let Some(idx) = line.find("http://") {
                    let rest = &line[idx + "http://".len()..];
                    bound = Some(rest.trim().to_string());
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }
    let addr = bound.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("server did not announce bound address");
    });

    let url = format!("http://{addr}/rpc/list_repos");
    let body = "{}";

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let unauth = client
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .expect("send unauth");
        let unauth_status = unauth.status();

        let authed = client
            .post(&url)
            .header("content-type", "application/json")
            .bearer_auth(token)
            .body(body)
            .send()
            .await
            .expect("send authed");
        let authed_status = authed.status();
        let authed_body = authed.text().await.unwrap_or_default();

        (unauth_status, authed_status, authed_body)
    });

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader_thread.join();

    let (unauth, authed, body) = outcome;
    assert_eq!(unauth.as_u16(), 401, "unauth should be 401");
    assert_eq!(authed.as_u16(), 200, "authed should be 200, body={body}");
    assert!(body.contains("\"repos\""), "list_repos body: {body}");
}

#[test]
fn serve_refuses_without_configured_token() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    let out = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--require-token",
        ])
        .timeout(Duration::from_secs(5))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--init-token"),
        "expected hint about --init-token, got: {stderr}"
    );
}

/// Non-loopback bind without `--require-token` must refuse to boot.
/// The footgun guard exists so a careless `--bind 0.0.0.0:...` cannot
/// accidentally expose an unauthenticated core to other hosts.
#[test]
fn serve_refuses_non_loopback_without_require_token() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");

    let out = TestCommand::new(BIN)
        .args([
            "--secrets-file",
            secrets.to_str().unwrap(),
            "serve",
            "--bind",
            "0.0.0.0:0",
        ])
        .timeout(Duration::from_secs(5))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--require-token"),
        "expected hint about --require-token, got: {stderr}"
    );
}

/// Loopback bind without a configured token must succeed with auth
/// disabled — the zero-paste first-run path the demo depends on.
#[test]
fn serve_loopback_no_token_boots_and_accepts_unauthed_request() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut child = Command::new(BIN)
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
        .expect("spawn codeless serve");

    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let _reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line.clone()).is_err() {
                break;
            }
            if line.contains("listening on http://") {
                break;
            }
        }
    });

    let mut bound = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(line) => {
                if let Some(idx) = line.find("http://") {
                    bound = Some(line[idx + "http://".len()..].trim().to_string());
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }
    let addr = bound.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("server did not announce bound address");
    });

    let url = format!("http://{addr}/rpc/list_repos");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let status = rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        client
            .post(&url)
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .expect("send unauth")
            .status()
    });
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        status.as_u16(),
        200,
        "expected 200 on loopback without token, got {status}"
    );
}
