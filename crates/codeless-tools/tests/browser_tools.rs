//! End-to-end tests for the browser.* tool surface using the same
//! bash fake-sidecar harness as browser_lifecycle.rs.
//!
//! These tests prove Tool → BrowserManager → sidecar wiring composes.
//! No real Playwright is involved; the fake echoes structured
//! responses keyed on the method name.

use std::path::PathBuf;
use std::sync::Arc;

use codeless_tools::browser::{BrowserManager, BrowserManagerConfig};
use codeless_tools::policy::{AllowlistFile, NetworkMode};
use codeless_tools::testing::fake_ctx_builder;
use codeless_tools::tools::{
    BrowserNavigateTool, BrowserReadTool, BrowserSessionCloseTool, BrowserSessionListTool,
    BrowserSessionOpenTool,
};
use codeless_tools::{Tool, ToolError};
use serde_json::json;
use tempfile::TempDir;

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
    session.create)
      printf '{"id":%s,"ok":true,"result":{"sessionId":"sess-fake"}}\n' "$id"
      ;;
    session.close)
      printf '{"id":%s,"ok":true,"result":{"closed":true}}\n' "$id"
      ;;
    session.list)
      printf '{"id":%s,"ok":true,"result":{"sessions":[]}}\n' "$id"
      ;;
    page.goto)
      printf '{"id":%s,"ok":true,"result":{"page_id":"page-fake","status":200,"url":"https://example.com/landed"}}\n' "$id"
      ;;
    page.read)
      printf '{"id":%s,"ok":true,"result":{"title":"hi","html":"<p>hello</p>","byte_length":12,"truncated":false,"final_url":"https://example.com/landed"}}\n' "$id"
      ;;
    *)
      printf '{"id":%s,"ok":false,"error":{"code":"not_found","message":"unknown method"}}\n' "$id"
      ;;
  esac
done
"#;

struct Harness {
    _tmp: TempDir,
    mgr: Arc<BrowserManager>,
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
    let bash = which_bash();
    let config =
        BrowserManagerConfig::new(bash, script, tmp.path().join("browsers")).with_node_args(vec![]);
    Harness {
        _tmp: tmp,
        mgr: BrowserManager::new(config),
    }
}

fn which_bash() -> PathBuf {
    for c in ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"] {
        if std::path::Path::new(c).exists() {
            return PathBuf::from(c);
        }
    }
    panic!("bash not found");
}

#[tokio::test]
async fn session_open_returns_session_id() {
    let h = fake_harness();
    let tool = BrowserSessionOpenTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool.call(&ctx.ctx, json!({})).await.expect("ok");
    assert_eq!(result["sessionId"], "sess-fake");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn session_list_and_close_dispatch() {
    let h = fake_harness();
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();

    let listed = BrowserSessionListTool::new(h.mgr.clone())
        .call(&ctx.ctx, json!({}))
        .await
        .expect("list ok");
    assert!(listed.get("sessions").is_some());

    let closed = BrowserSessionCloseTool::new(h.mgr.clone())
        .call(&ctx.ctx, json!({ "session_id": "x" }))
        .await
        .expect("close ok");
    assert_eq!(closed["closed"], true);

    h.mgr.shutdown().await;
}

#[tokio::test]
async fn navigate_enforces_allowlist_before_hitting_sidecar() {
    let h = fake_harness();
    let tool = BrowserNavigateTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder()
        .network_mode(NetworkMode::Allowlist)
        .allowlist(AllowlistFile::with_hosts(["example.com"]))
        .build();
    let err = tool
        .call(
            &ctx.ctx,
            json!({ "session_id": "s", "url": "https://blocked.test/foo" }),
        )
        .await
        .expect_err("denied");
    assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn navigate_dispatches_when_allowed() {
    let h = fake_harness();
    let tool = BrowserNavigateTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "session_id": "s", "url": "https://example.com" }),
        )
        .await
        .expect("ok");
    assert_eq!(result["page_id"], "page-fake");
    assert_eq!(result["status"], 200);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn read_returns_html_payload() {
    let h = fake_harness();
    let tool = BrowserReadTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "page-fake" }))
        .await
        .expect("ok");
    assert_eq!(result["html"], "<p>hello</p>");
    assert_eq!(result["title"], "hi");
    assert_eq!(result["truncated"], false);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn read_missing_page_id_is_invalid_args() {
    let h = fake_harness();
    let tool = BrowserReadTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let err = tool
        .call(&ctx.ctx, json!({}))
        .await
        .expect_err("missing page_id");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn cancellation_short_circuits_browser_tools() {
    let h = fake_harness();
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    ctx.cancel.cancel();

    let open = BrowserSessionOpenTool::new(h.mgr.clone());
    let err = open.call(&ctx.ctx, json!({})).await.expect_err("cancelled");
    assert!(matches!(err, ToolError::Cancelled), "got {err:?}");
    h.mgr.shutdown().await;
}
