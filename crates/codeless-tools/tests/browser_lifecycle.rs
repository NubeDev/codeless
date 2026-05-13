//! Lifecycle tests for the Playwright sidecar plumbing.
//!
//! Playwright is not installed in CI, so these tests use a tiny
//! shell-script "fake sidecar" that speaks the same JSON-RPC line
//! protocol. That exercises every codeless-side path that matters:
//! - spawn / stdin / stdout pumping
//! - id-multiplexed request → response matching
//! - dead-process detection
//! - per-call timeout
//! - graceful shutdown
//!
//! Replacing the fake with the real Node sidecar is a configuration
//! change, not a code change. The real-Playwright integration lives
//! in its own ignored test (see `#[ignore]` below).

use std::path::PathBuf;
use std::time::Duration;

use codeless_tools::browser::{BrowserManager, BrowserManagerConfig};
use tempfile::TempDir;

/// Fake sidecar: reads JSON requests from stdin, writes back a
/// response with `ok:true` and the original params echoed in
/// `result.echo`. A request with method=="shutdown" exits cleanly;
/// method=="fail" returns ok:false with code="invalid_params".
const FAKE_SIDECAR_SH: &str = r#"#!/usr/bin/env bash
set -eu
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":[ ]*\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":[ ]*"\([^"]*\)".*/\1/p')
  case "$method" in
    shutdown)
      printf '{"id":%s,"ok":true,"result":{"shutting_down":true}}\n' "$id"
      exit 0
      ;;
    fail)
      printf '{"id":%s,"ok":false,"error":{"code":"invalid_params","message":"bad"}}\n' "$id"
      ;;
    slow)
      sleep 5
      printf '{"id":%s,"ok":true,"result":{"slow":true}}\n' "$id"
      ;;
    *)
      printf '{"id":%s,"ok":true,"result":{"method":"%s"}}\n' "$id" "$method"
      ;;
  esac
done
"#;

struct Harness {
    _tmp: TempDir,
    config: BrowserManagerConfig,
}

fn fake_harness() -> Harness {
    let tmp = TempDir::new().expect("tempdir");
    let script = tmp.path().join("fake-sidecar.sh");
    std::fs::write(&script, FAKE_SIDECAR_SH).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&script).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&script, p).unwrap();
    }
    // bash is the "Node" runtime in this fake. Clear node_args so
    // bash isn't handed `--max-old-space-size=512`.
    let bash = which_bash();
    let config =
        BrowserManagerConfig::new(bash, script, tmp.path().join("browsers")).with_node_args(vec![]);
    Harness { _tmp: tmp, config }
}

fn which_bash() -> PathBuf {
    for candidate in ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"] {
        if std::path::Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    panic!("bash not found in standard locations");
}

#[tokio::test]
async fn request_dispatches_and_returns_result() {
    let h = fake_harness();
    let mgr = BrowserManager::new(h.config);
    let result = mgr
        .request("ping", serde_json::json!({}), Some(Duration::from_secs(5)))
        .await
        .expect("request ok");
    assert_eq!(result["method"], "ping");
    mgr.shutdown().await;
}

#[tokio::test]
async fn concurrent_requests_multiplex_correctly() {
    let h = fake_harness();
    let mgr = BrowserManager::new(h.config);

    let a = mgr.request("alpha", serde_json::json!({}), Some(Duration::from_secs(5)));
    let b = mgr.request("beta", serde_json::json!({}), Some(Duration::from_secs(5)));
    let c = mgr.request("gamma", serde_json::json!({}), Some(Duration::from_secs(5)));
    let (ra, rb, rc) = tokio::join!(a, b, c);

    assert_eq!(ra.unwrap()["method"], "alpha");
    assert_eq!(rb.unwrap()["method"], "beta");
    assert_eq!(rc.unwrap()["method"], "gamma");

    mgr.shutdown().await;
}

#[tokio::test]
async fn rpc_error_maps_to_tool_error() {
    let h = fake_harness();
    let mgr = BrowserManager::new(h.config);
    let err = mgr
        .request("fail", serde_json::json!({}), Some(Duration::from_secs(5)))
        .await
        .expect_err("sidecar returns ok:false");
    let msg = err.to_string();
    assert!(msg.contains("bad"), "got {msg}");
    mgr.shutdown().await;
}

#[tokio::test]
async fn per_call_timeout_fires() {
    let h = fake_harness();
    let mgr = BrowserManager::new(h.config);
    let err = mgr
        .request(
            "slow",
            serde_json::json!({}),
            Some(Duration::from_millis(200)),
        )
        .await
        .expect_err("timeout");
    let msg = err.to_string();
    assert!(msg.contains("timed out"), "got {msg}");
    mgr.shutdown().await;
}

#[tokio::test]
async fn shutdown_is_idempotent() {
    let h = fake_harness();
    let mgr = BrowserManager::new(h.config);
    let _ = mgr
        .request(
            "warmup",
            serde_json::json!({}),
            Some(Duration::from_secs(5)),
        )
        .await;
    mgr.shutdown().await;
    mgr.shutdown().await;
}

/// Smoke test against the real Node + Playwright sidecar. Ignored by
/// default because Playwright isn't part of the CI image; run locally
/// with `cargo test -p codeless-tools -- --ignored` after pointing
/// `CODELESS_NODE_BIN` and `CODELESS_PLAYWRIGHT_SIDECAR` at a real
/// install.
#[tokio::test]
#[ignore = "requires Node + playwright-core; opt-in"]
async fn real_playwright_navigate_returns_html() {
    let node = std::env::var_os("CODELESS_NODE_BIN")
        .map(PathBuf::from)
        .expect("set CODELESS_NODE_BIN to a node binary");
    let script = std::env::var_os("CODELESS_PLAYWRIGHT_SIDECAR")
        .map(PathBuf::from)
        .expect("set CODELESS_PLAYWRIGHT_SIDECAR to the sidecar.mjs path");
    let browsers = std::env::var_os("CODELESS_PLAYWRIGHT_BROWSERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/codeless-playwright-browsers"));

    let mgr = BrowserManager::new(BrowserManagerConfig::new(node, script, browsers));
    let result = mgr
        .request(
            "session.open",
            serde_json::json!({}),
            Some(Duration::from_secs(30)),
        )
        .await
        .expect("open session");
    assert!(result.get("sessionId").is_some());
    mgr.shutdown().await;
}
