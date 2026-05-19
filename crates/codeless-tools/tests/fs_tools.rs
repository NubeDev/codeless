//! End-to-end exercise of the assistant fs tools through the
//! shared `ToolRegistry` the planner advertises to its runner. The
//! per-tool unit tests already cover happy-path and sandbox-reject
//! paths in isolation; this test pins the registration helper and
//! the full dispatch sequence (list -> read -> search) the way the
//! planner-driven runner will use them.
//!
//! "MockRunner" in the stage description refers to driving the tool
//! surface without spawning a real CLI runner; the equivalent here
//! is invoking `Tool::call` through `ToolRegistry::get` exactly as
//! the codeless-mcp dispatch path does (mirrors how
//! `codeless-mcp/tests/plugin_mcp_e2e.rs` drives plugin tools).

use std::sync::Arc;

use codeless_tools::fs::{register_assistant_thread_read_tools, Sandbox};
use codeless_tools::testing::fake_ctx;
use codeless_tools::{ToolError, ToolRegistry};
use serde_json::{json, Value};
use tempfile::TempDir;

async fn fixture_registry(root: &TempDir) -> ToolRegistry {
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let mut registry = ToolRegistry::new();
    register_assistant_thread_read_tools(&mut registry, sandbox);
    registry
}

#[tokio::test]
async fn registers_exactly_the_three_read_tools() {
    let root = TempDir::new().unwrap();
    let registry = fixture_registry(&root).await;
    let mut names: Vec<&str> = registry.names().collect();
    names.sort();
    assert_eq!(names, vec!["fs.list", "fs.read", "fs.search"]);
}

#[tokio::test]
async fn list_then_read_then_search_returns_workspace_content() {
    let root = TempDir::new().unwrap();
    tokio::fs::write(
        root.path().join("notes.txt"),
        "needle on line one\nfiller\n",
    )
    .await
    .unwrap();
    tokio::fs::create_dir(root.path().join("src"))
        .await
        .unwrap();
    tokio::fs::write(root.path().join("src/lib.rs"), "fn needle() {}\n")
        .await
        .unwrap();

    let registry = fixture_registry(&root).await;
    let harness = fake_ctx();

    let list_out = registry
        .get("fs.list")
        .unwrap()
        .call(&harness.ctx, json!({ "path": "." }))
        .await
        .unwrap();
    let names: Vec<&str> = list_out
        .get("entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|e| e.get("name").and_then(Value::as_str).unwrap())
        .collect();
    assert_eq!(names, vec!["notes.txt", "src"]);

    let read_out = registry
        .get("fs.read")
        .unwrap()
        .call(&harness.ctx, json!({ "path": "notes.txt" }))
        .await
        .unwrap();
    assert_eq!(
        read_out.get("content").and_then(Value::as_str),
        Some("needle on line one\nfiller\n"),
    );

    let search_out = registry
        .get("fs.search")
        .unwrap()
        .call(&harness.ctx, json!({ "query": "needle" }))
        .await
        .unwrap();
    let matches = search_out.get("matches").and_then(Value::as_array).unwrap();
    // Two files match; both should land. Ordering is traversal-
    // dependent so we assert on the set, not the sequence.
    let paths: std::collections::HashSet<&str> = matches
        .iter()
        .map(|m| m.get("path").and_then(Value::as_str).unwrap())
        .collect();
    assert!(paths.contains("notes.txt"));
    assert!(paths.contains("src/lib.rs"));
    assert_eq!(
        search_out.get("truncated").and_then(Value::as_bool),
        Some(false),
    );
}

#[tokio::test]
async fn every_tool_rejects_paths_outside_the_root() {
    // The sandbox is the same instance across the three tools, so
    // a single escape attempt failing on each tool is the test the
    // registration helper actually buys us: a regression that
    // unwires the sandbox for one tool would slip past per-tool
    // tests but not this one.
    let root = TempDir::new().unwrap();
    let registry = fixture_registry(&root).await;
    let harness = fake_ctx();

    for (tool_name, args) in [
        ("fs.list", json!({ "path": "/etc" })),
        ("fs.read", json!({ "path": "/etc/passwd" })),
        ("fs.search", json!({ "query": "x", "path": "/etc" })),
    ] {
        let err = registry
            .get(tool_name)
            .unwrap()
            .call(&harness.ctx, args)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::Denied(_)),
            "{tool_name} should reject absolute path; got {err:?}",
        );
    }

    for (tool_name, args) in [
        ("fs.list", json!({ "path": ".." })),
        ("fs.read", json!({ "path": "../secret" })),
        ("fs.search", json!({ "query": "x", "path": "../secret" })),
    ] {
        let err = registry
            .get(tool_name)
            .unwrap()
            .call(&harness.ctx, args)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::Denied(_)),
            "{tool_name} should reject parent traversal; got {err:?}",
        );
    }
}
