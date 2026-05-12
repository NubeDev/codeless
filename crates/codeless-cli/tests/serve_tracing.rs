//! Confirms `codeless serve` emits a tracing line per HTTP request
//! (via tower-http's TraceLayer) so operators can debug a running
//! core with just stderr. The default filter is
//! `info,tower_http=info` so the line shows up without any
//! `RUST_LOG` override; tests pin `RUST_LOG=debug` to also catch the
//! request-finished event.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use codeless_adapters_host::SecretStore;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");
const TOKEN: &str = "tracing-test-token";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_logs_request_to_stderr() {
    let dir = TempDir::new().unwrap();
    let secrets = dir.path().join("secrets.toml");
    let db = dir.path().join("codeless.db");

    let mut store = SecretStore::open(&secrets).unwrap();
    store.set("core_bearer_token", TOKEN).unwrap();
    store.save().unwrap();

    let mut server = Command::new(BIN)
        .env("RUST_LOG", "info,tower_http=debug")
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
            let _ = tx.send(line);
        }
    });

    // Wait for the listening line.
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

    // Hit /healthz to generate a request log line. /healthz is
    // unauthenticated so no token plumbing is needed.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Drain stderr for up to 2s looking for the tower-http trace
    // span. The exact format is "started processing request" /
    // "finished processing request" — assert on the request span
    // marker keywords that survive across tower-http minor versions.
    let mut saw_request_log = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
            if (line.contains("request") || line.contains("response"))
                && (line.contains("/healthz") || line.contains("GET"))
            {
                saw_request_log = true;
                break;
            }
        }
    }

    let _ = server.kill();
    let _ = server.wait();
    let _ = reader.join();

    assert!(saw_request_log, "no request log line surfaced on stderr");
}
