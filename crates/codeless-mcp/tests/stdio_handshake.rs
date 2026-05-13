//! End-to-end test against the codeless-mcp binary over stdio.
//!
//! Spawns the binary as a child process, drives the MCP handshake,
//! lists tools, calls one. Proves the whole stack composes: rmcp
//! transport -> CodelessMcpHandler -> ToolRegistry -> Tool::call.

use rmcp::model::CallToolRequestParams;
use rmcp::service::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use tokio::process::Command;

fn server_binary() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> points at the just-built test binary,
    // which guarantees we run the same code under test rather than
    // a stale install.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_codeless-mcp"))
}

#[tokio::test]
async fn handshake_lists_default_tools() {
    let bin = server_binary();
    let client = ()
        .serve(
            TokioChildProcess::new(Command::new(&bin).configure(|cmd| {
                cmd.env(
                    "CODELESS_WORKTREE_ROOT",
                    std::env::temp_dir().to_string_lossy().as_ref(),
                );
            }))
            .expect("spawn codeless-mcp"),
        )
        .await
        .expect("mcp init handshake");

    let info = client.peer_info();
    assert!(info.is_some(), "server returned no info");

    let tools = client.list_tools(Default::default()).await.expect("list");
    let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    assert!(
        names.iter().any(|n| n == "codeless.browse.fetch"),
        "expected codeless.browse.fetch in {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "codeless.http.request"),
        "expected codeless.http.request in {names:?}"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn call_browse_fetch_with_network_none_returns_denied_error() {
    let bin = server_binary();
    let client = ()
        .serve(
            TokioChildProcess::new(Command::new(&bin).configure(|cmd| {
                cmd.env(
                    "CODELESS_WORKTREE_ROOT",
                    std::env::temp_dir().to_string_lossy().as_ref(),
                );
            }))
            .expect("spawn codeless-mcp"),
        )
        .await
        .expect("mcp init handshake");

    // Default server context has NetworkMode::None — the tool
    // should refuse to fetch and surface a structured error.
    let result = client
        .call_tool(
            CallToolRequestParams::new("codeless.browse.fetch").with_arguments(
                serde_json::json!({ "url": "https://example.com" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("tool call");

    assert_eq!(result.is_error, Some(true), "expected is_error: true");
    // Structured content should carry the error kind for the LLM
    // to pattern-match.
    let structured = result.structured_content.expect("structured");
    let kind = structured["error"]["kind"].as_str().expect("kind string");
    assert_eq!(kind, "denied", "got structured={structured}");

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn unknown_tool_call_returns_protocol_error() {
    let bin = server_binary();
    let client = ()
        .serve(
            TokioChildProcess::new(Command::new(&bin).configure(|cmd| {
                cmd.env(
                    "CODELESS_WORKTREE_ROOT",
                    std::env::temp_dir().to_string_lossy().as_ref(),
                );
            }))
            .expect("spawn codeless-mcp"),
        )
        .await
        .expect("mcp init handshake");

    let err = client
        .call_tool(CallToolRequestParams::new("codeless.nope.nope"))
        .await
        .expect_err("unknown tool");
    let msg = err.to_string();
    assert!(
        msg.contains("nope") || msg.contains("unknown"),
        "got err={msg}"
    );

    client.cancel().await.expect("clean shutdown");
}
