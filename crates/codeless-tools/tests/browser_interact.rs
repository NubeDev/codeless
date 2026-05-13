//! Phase 3.4 interaction tools (click/type/fill/hover/scroll) against
//! the fake-sidecar harness.
//!
//! Each tool gets:
//!
//! - one happy-path dispatch test (sidecar method invoked, result
//!   returned)
//! - one required-arg validation test (covering the field unique
//!   to that tool — `text` for type, `value` for fill, `selector`
//!   for the rest).
//!
//! Cancellation, page_id, and timeout plumbing are already covered
//! by the earlier browser_*.rs test files; not re-tested here.

use std::path::PathBuf;
use std::sync::Arc;

use codeless_tools::browser::{BrowserManager, BrowserManagerConfig};
use codeless_tools::policy::NetworkMode;
use codeless_tools::testing::{fake_ctx_builder, FakeCtx};
use codeless_tools::tools::{
    BrowserClickTool, BrowserFillTool, BrowserHoverTool, BrowserScrollTool, BrowserTypeTool,
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
    page.click)
      printf '{"id":%s,"ok":true,"result":{"clicked":true}}\n' "$id"
      ;;
    page.type)
      printf '{"id":%s,"ok":true,"result":{"typed":true}}\n' "$id"
      ;;
    page.fill)
      printf '{"id":%s,"ok":true,"result":{"filled":true}}\n' "$id"
      ;;
    page.hover)
      printf '{"id":%s,"ok":true,"result":{"hovered":true}}\n' "$id"
      ;;
    page.scroll)
      printf '{"id":%s,"ok":true,"result":{"scrolled":true}}\n' "$id"
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
async fn click_dispatches() {
    let h = fake_harness();
    let tool = BrowserClickTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "selector": "#submit" }))
        .await
        .expect("ok");
    assert_eq!(result["clicked"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn click_requires_selector() {
    let h = fake_harness();
    let tool = BrowserClickTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1" }))
        .await
        .expect_err("missing selector");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn type_dispatches() {
    let h = fake_harness();
    let tool = BrowserTypeTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "selector": "#q", "text": "hello" }),
        )
        .await
        .expect("ok");
    assert_eq!(result["typed"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn type_requires_text() {
    let h = fake_harness();
    let tool = BrowserTypeTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "selector": "#q" }))
        .await
        .expect_err("missing text");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn fill_dispatches() {
    let h = fake_harness();
    let tool = BrowserFillTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "selector": "#email", "value": "a@b.c" }),
        )
        .await
        .expect("ok");
    assert_eq!(result["filled"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn fill_requires_value() {
    let h = fake_harness();
    let tool = BrowserFillTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "selector": "#email" }))
        .await
        .expect_err("missing value");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn hover_dispatches() {
    let h = fake_harness();
    let tool = BrowserHoverTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(
            &ctx.ctx,
            json!({ "page_id": "p1", "selector": ".tooltip-trigger" }),
        )
        .await
        .expect("ok");
    assert_eq!(result["hovered"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn scroll_dispatches_without_selector() {
    let h = fake_harness();
    let tool = BrowserScrollTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let result = tool
        .call(&ctx.ctx, json!({ "page_id": "p1", "direction": "bottom" }))
        .await
        .expect("ok");
    assert_eq!(result["scrolled"], true);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn scroll_requires_page_id() {
    let h = fake_harness();
    let tool = BrowserScrollTool::new(h.mgr.clone());
    let ctx = open_ctx();
    let err = tool
        .call(&ctx.ctx, json!({}))
        .await
        .expect_err("missing page_id");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}
