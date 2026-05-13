//! browser.crawl end-to-end against a fake sidecar that serves
//! HTML with discoverable links.
//!
//! The fake increments a page counter per page.read call and serves
//! pre-canned HTML keyed on the requested URL. This lets us drive
//! a real BFS through the crawl tool without hitting a network.

use std::path::PathBuf;
use std::sync::Arc;

use codeless_tools::browser::{BrowserManager, BrowserManagerConfig};
use codeless_tools::policy::{AllowlistFile, NetworkMode};
use codeless_tools::testing::fake_ctx_builder;
use codeless_tools::tools::BrowserCrawlTool;
use codeless_tools::{Tool, ToolError};
use serde_json::json;
use tempfile::TempDir;

// Fake sidecar serving three linked pages on a single host:
//   /a links to /b
//   /b links to /c
//   /c has no further links
// page.goto returns the requested URL; page.read returns canned
// HTML keyed on the latest URL we saw.
const FAKE_SIDECAR_SH: &str = r#"#!/usr/bin/env bash
set -eu
last_url=""
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":[ ]*\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":[ ]*"\([^"]*\)".*/\1/p')
  case "$method" in
    shutdown)
      printf '{"id":%s,"ok":true,"result":{"shutting_down":true}}\n' "$id"
      exit 0
      ;;
    session.create)
      printf '{"id":%s,"ok":true,"result":{"session_id":"sess-1"}}\n' "$id"
      ;;
    session.close)
      printf '{"id":%s,"ok":true,"result":{"closed":true}}\n' "$id"
      ;;
    page.goto)
      url=$(printf '%s' "$line" | sed -n 's/.*"url":[ ]*"\([^"]*\)".*/\1/p')
      last_url="$url"
      printf '{"id":%s,"ok":true,"result":{"page_id":"page-1","status":200,"url":"%s"}}\n' "$id" "$url"
      ;;
    page.read)
      case "$last_url" in
        *"/a"*)
          printf '{"id":%s,"ok":true,"result":{"title":"A","html":"<a href=\\"/b\\">go-b</a>","final_url":"https://e.test/a","byte_length":50,"truncated":false}}\n' "$id"
          ;;
        *"/b"*)
          printf '{"id":%s,"ok":true,"result":{"title":"B","html":"<a href=\\"/c\\">go-c</a>","final_url":"https://e.test/b","byte_length":50,"truncated":false}}\n' "$id"
          ;;
        *"/c"*)
          printf '{"id":%s,"ok":true,"result":{"title":"C","html":"<p>leaf</p>","final_url":"https://e.test/c","byte_length":30,"truncated":false}}\n' "$id"
          ;;
        *)
          printf '{"id":%s,"ok":true,"result":{"title":"?","html":"","final_url":"%s","byte_length":0,"truncated":false}}\n' "$id" "$last_url"
          ;;
      esac
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

#[tokio::test]
async fn crawl_walks_depth_1_by_default() {
    let h = fake_harness();
    let tool = BrowserCrawlTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool
        .call(&ctx.ctx, json!({ "url": "https://e.test/a" }))
        .await
        .expect("ok");
    let pages = result["pages"].as_array().expect("pages array");
    // depth 0: /a (linked to /b)
    // depth 1: /b (linked to /c; not followed because max_depth=1)
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["url"], "https://e.test/a");
    assert_eq!(pages[0]["depth"], 0);
    assert_eq!(pages[1]["depth"], 1);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn crawl_respects_max_pages() {
    let h = fake_harness();
    let tool = BrowserCrawlTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool
        .call(
            &ctx.ctx,
            json!({
                "url": "https://e.test/a",
                "max_depth": 5,
                "max_pages": 1
            }),
        )
        .await
        .expect("ok");
    assert_eq!(result["pages_crawled"], 1);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn crawl_full_chain_with_higher_depth() {
    let h = fake_harness();
    let tool = BrowserCrawlTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let result = tool
        .call(
            &ctx.ctx,
            json!({
                "url": "https://e.test/a",
                "max_depth": 3,
                "max_pages": 10
            }),
        )
        .await
        .expect("ok");
    // /a (d=0) -> /b (d=1) -> /c (d=2). leaf at /c.
    assert_eq!(result["pages_crawled"], 3);
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn crawl_denies_disallowed_start_url() {
    let h = fake_harness();
    let tool = BrowserCrawlTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder()
        .network_mode(NetworkMode::Allowlist)
        .allowlist(AllowlistFile::with_hosts(["other.test"]))
        .build();
    let err = tool
        .call(&ctx.ctx, json!({ "url": "https://e.test/a" }))
        .await
        .expect_err("denied");
    assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    h.mgr.shutdown().await;
}

#[tokio::test]
async fn crawl_missing_url_is_invalid_args() {
    let h = fake_harness();
    let tool = BrowserCrawlTool::new(h.mgr.clone());
    let ctx = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let err = tool.call(&ctx.ctx, json!({})).await.expect_err("no url");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    h.mgr.shutdown().await;
}
