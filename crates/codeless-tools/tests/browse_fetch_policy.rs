//! Network-policy gating for browse.fetch.
//!
//! Tests in this file deliberately do NOT hit the network. They
//! verify that the tool denies the request before reqwest is built
//! whenever policy doesn't permit egress — that boundary is the
//! load-bearing piece codeless owns.

use codeless_tools::policy::{AllowlistFile, NetworkMode};
use codeless_tools::testing::fake_ctx_builder;
use codeless_tools::tools::BrowseFetchTool;
use codeless_tools::{Tool, ToolError};
use serde_json::json;

#[tokio::test]
async fn network_none_denies_fetch() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::None).build();
    let tool = BrowseFetchTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "https://example.com" }))
        .await
        .expect_err("network is disabled");
    assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
}

#[tokio::test]
async fn allowlist_denies_unlisted_host() {
    let harness = fake_ctx_builder()
        .network_mode(NetworkMode::Allowlist)
        .allowlist(AllowlistFile::with_hosts(["example.com"]))
        .build();
    let tool = BrowseFetchTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "https://other.example.org" }))
        .await
        .expect_err("host not in allowlist");
    assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
}

#[tokio::test]
async fn missing_url_arg_is_invalid_args() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let tool = BrowseFetchTool::new();
    let err = tool
        .call(&harness.ctx, json!({}))
        .await
        .expect_err("missing url");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
}

#[tokio::test]
async fn url_without_host_is_invalid_args() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let tool = BrowseFetchTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "" }))
        .await
        .expect_err("empty URL has no host");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
}

#[tokio::test]
async fn cancellation_before_call_short_circuits() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    harness.cancel.cancel();
    let tool = BrowseFetchTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "https://example.com" }))
        .await
        .expect_err("cancelled before egress");
    assert!(matches!(err, ToolError::Cancelled), "got {err:?}");
}
