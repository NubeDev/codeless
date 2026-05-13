//! Phase 3.3 tools end-to-end against the bash fake-sidecar.
//!
//! Covers screenshot (with and without save_to), eval (incl. the
//! 8 KiB cap), extract, wait, cookies.

use std::path::PathBuf;
use std::sync::Arc;

use codeless_tools::browser::{BrowserManager, BrowserManagerConfig};
use codeless_tools::policy::NetworkMode;
use codeless_tools::testing::{fake_ctx_builder, FakeCtx};
use codeless_tools::tools::{
    BrowserCookiesTool, BrowserEvalTool, BrowserExtractTool, BrowserScreenshotTool, BrowserWaitTool,
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
    page.screenshot)
      if printf '%s' "$line" | grep -q '"save_to_path"'; then
        path=$(printf '%s' "$line" | sed -n 's/.*"save_to_path":[ ]*"\([^"]*\)".*/\1/p')
        printf '{"id":%s,"ok":true,"result":{"saved_to":"%s","bytes":1234}}\n' "$id" "$path"
      else
        printf '{"id":%s,"ok":true,"result":{"image_b64":"PNG-BASE64","bytes":1234}}\n' "$id"
      fi
      ;;
    page.eval)
      printf '{"id":%s,"ok":true,"result":{"value":42}}\n' "$id"
      ;;
    page.extract)
      printf '{"id":%s,"ok":true,"result":{"title":"H","body":"B"}}\n' "$id"
      ;;
    page.wait_for)
      printf '{"id":%s,"ok":true,"result":{"matched":true}}\n' "$id"
      ;;
    page.cookies)
      printf '{"id":%s,"ok":true,"result":{"cookies":[]}}\n' "$id"
      ;;
    *)
      printf '{"id":%s,"ok":false,"error":{"code":"not_found","message":"unknown"}}\n' "$id"
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
    std::fs::write(&script, FAKE_SIDECAR_SH).expect("write");
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

fn open_ctx() -> FakeCtx {
    fake_ctx_builder().network_mode(NetworkMode::Open).build()
}

#[tokio::test]
async fn screenshot_returns_base64_when_no_save_to() {
    let h = fake_harness();
    let tool = BrowserScreenshotTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "p1" }))
        .await
        .expect("ok");
    assert_eq!(result["image_b64"], "PNG-BASE64");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn screenshot_rewrites_save_to_into_worktree_absolute() {
    let h = fake_harness();
    let tool = BrowserScreenshotTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "save_to": "out/img.png" }),
        )
        .await
        .expect("ok");

    let saved = result["saved_to"].as_str().expect("saved_to is a string");
    let expected = ctx.ctx.worktree_root().join("out/img.png");
    assert_eq!(saved, expected.to_string_lossy());
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn screenshot_rejects_absolute_save_to() {
    let h = fake_harness();
    let tool = BrowserScreenshotTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "save_to": "/etc/evil.png" }),
        )
        .await
        .expect_err("rejected");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn screenshot_rejects_parent_traversal_save_to() {
    let h = fake_harness();
    let tool = BrowserScreenshotTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "save_to": "../oops.png" }),
        )
        .await
        .expect_err("rejected");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn eval_dispatches() {
    let h = fake_harness();
    let tool = BrowserEvalTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "expression": "1 + 1" }))
        .await
        .expect("ok");
    assert_eq!(result["value"], 42);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn eval_rejects_oversized_expression() {
    let h = fake_harness();
    let tool = BrowserEvalTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let big = "x".repeat(9 * 1024);
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "expression": big }))
        .await
        .expect_err("exceeds cap");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn eval_requires_expression() {
    let h = fake_harness();
    let tool = BrowserEvalTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1" }))
        .await
        .expect_err("missing");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn extract_requires_selectors() {
    let h = fake_harness();
    let tool = BrowserExtractTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1" }))
        .await
        .expect_err("missing selectors");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn extract_dispatches() {
    let h = fake_harness();
    let tool = BrowserExtractTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "selectors": { "title": "h1" } }),
        )
        .await
        .expect("ok");
    assert_eq!(result["title"], "H");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn wait_dispatches() {
    let h = fake_harness();
    let tool = BrowserWaitTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "selector": ".loaded", "state": "visible" }),
        )
        .await
        .expect("ok");
    assert_eq!(result["matched"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn cookies_requires_action() {
    let h = fake_harness();
    let tool = BrowserCookiesTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1" }))
        .await
        .expect_err("missing action");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn cookies_dispatches() {
    let h = fake_harness();
    let tool = BrowserCookiesTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "action": "get" }))
        .await
        .expect("ok");
    assert!(result.get("cookies").is_some());
    h.mgr.shutdown().await;
}
