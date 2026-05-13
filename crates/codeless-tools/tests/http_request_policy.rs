//! Network-policy and arg-validation gating for http.request.
//!
//! No network is touched: every test exercises a denial or
//! input-validation path that returns before reqwest is built.

use codeless_tools::policy::{AllowlistFile, NetworkMode};
use codeless_tools::testing::fake_ctx_builder;
use codeless_tools::tools::HttpRequestTool;
use codeless_tools::{Tool, ToolError};
use serde_json::json;

#[tokio::test]
async fn network_none_denies_request() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::None).build();
    let tool = HttpRequestTool::new();
    let err = tool
        .call(
            &harness.ctx,
            json!({ "url": "https://example.com", "method": "POST" }),
        )
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
    let tool = HttpRequestTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "https://other.example.org" }))
        .await
        .expect_err("host not in allowlist");
    assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
}

#[tokio::test]
async fn unsupported_method_is_invalid_args() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let tool = HttpRequestTool::new();
    let err = tool
        .call(
            &harness.ctx,
            json!({ "url": "https://example.com", "method": "TRACE" }),
        )
        .await
        .expect_err("TRACE not allowed");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
}

#[tokio::test]
async fn non_string_header_value_is_invalid_args() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let tool = HttpRequestTool::new();
    let err = tool
        .call(
            &harness.ctx,
            json!({
                "url": "https://example.com",
                "headers": { "x-thing": 42 }
            }),
        )
        .await
        .expect_err("header value must be string");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
}

#[tokio::test]
async fn missing_url_arg_is_invalid_args() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    let tool = HttpRequestTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "method": "GET" }))
        .await
        .expect_err("missing url");
    assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
}

#[tokio::test]
async fn cancellation_before_call_short_circuits() {
    let harness = fake_ctx_builder().network_mode(NetworkMode::Open).build();
    harness.cancel.cancel();
    let tool = HttpRequestTool::new();
    let err = tool
        .call(&harness.ctx, json!({ "url": "https://example.com" }))
        .await
        .expect_err("cancelled before egress");
    assert!(matches!(err, ToolError::Cancelled), "got {err:?}");
}
