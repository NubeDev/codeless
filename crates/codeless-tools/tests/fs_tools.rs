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

use async_trait::async_trait;
use codeless_tools::fs::dispatch::test_helpers::{RecordedWrite, RecordingDispatcher};
use codeless_tools::fs::{
    register_assistant_thread_read_tools, register_assistant_thread_write_tools, JobScopeWrite,
    Sandbox, WorkspaceWrite, WriteDispatcher,
};
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

// ===========================================================================
// Stage 6: write tool registration and mode-gated dispatch.
// ---------------------------------------------------------------------------
// The three modes (`read-only` / `approve-edits` / `bypass`) live on
// `assistant_threads.mode`; the caller resolves the row, decides whether
// to invoke `register_assistant_thread_write_tools`, and chooses the
// dispatcher impl that backs the trait. The registration helper itself
// is mode-blind by design (D8 omits the tools entirely on `read-only`),
// so the per-mode assertions live at the caller's seam.

/// Stand-in for the caller's mode-resolution step. The registration
/// helper is invoked only for modes that grant write access; on
/// `read-only` the helper is intentionally not called, mirroring the
/// runtime's planned codepath in stage 7.
fn register_for_mode(
    registry: &mut ToolRegistry,
    sandbox: Arc<Sandbox>,
    mode: &str,
    dispatcher: Arc<dyn WriteDispatcher>,
) {
    match mode {
        "read-only" => {}
        "approve-edits" | "bypass" => {
            register_assistant_thread_write_tools(registry, sandbox, dispatcher);
        }
        other => panic!("unknown mode: {other}"),
    }
}

#[tokio::test]
async fn read_only_thread_does_not_register_write_tools() {
    // D8: a planner running on a `read-only` thread sees fs.list /
    // fs.read / fs.search and nothing else. Write tools must not
    // appear in the registry at all — surfacing them and then
    // rejecting would be noise.
    let root = TempDir::new().unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let mut registry = ToolRegistry::new();
    register_assistant_thread_read_tools(&mut registry, Arc::clone(&sandbox));
    register_for_mode(
        &mut registry,
        sandbox,
        "read-only",
        Arc::new(RecordingDispatcher::new()),
    );
    let mut names: Vec<&str> = registry.names().collect();
    names.sort();
    assert_eq!(
        names,
        vec!["fs.list", "fs.read", "fs.search"],
        "read-only must not register fs.write / fs.edit",
    );
}

#[tokio::test]
async fn approve_edits_mode_surfaces_write_through_dispatcher_with_before_diff() {
    // The approve-edits flow: a Tool call lands a `WorkspaceWrite`
    // on the dispatcher rather than touching disk. The runtime's
    // ApproveEditsWriteDispatcher (lands in stage 7) turns this into
    // an AssistantActionCard the user confirms via the existing
    // `confirm_assistant_action` dispatcher; this test pins the
    // contract the dispatcher relies on (before/after content
    // surface, abs path canonicalised, rel echoed verbatim).
    let root = TempDir::new().unwrap();
    tokio::fs::write(root.path().join("a.txt"), b"old")
        .await
        .unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let rec = Arc::new(RecordingDispatcher::new());
    let mut registry = ToolRegistry::new();
    register_for_mode(
        &mut registry,
        Arc::clone(&sandbox),
        "approve-edits",
        rec.clone(),
    );
    let harness = fake_ctx();
    registry
        .get("fs.write")
        .unwrap()
        .call(&harness.ctx, json!({ "path": "a.txt", "content": "new" }))
        .await
        .unwrap();

    // Disk untouched: the dispatcher is the seam, not the Tool.
    let on_disk = tokio::fs::read_to_string(root.path().join("a.txt"))
        .await
        .unwrap();
    assert_eq!(
        on_disk, "old",
        "approve-edits must not write through the Tool; the dispatcher decides",
    );
    let calls = rec.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        RecordedWrite::Workspace {
            rel_path,
            before,
            after,
            ..
        } => {
            assert_eq!(rel_path, "a.txt");
            assert_eq!(before.as_deref(), Some("old"));
            assert_eq!(after, "new");
        }
        other => panic!("expected Workspace, got {other:?}"),
    }
}

/// Bypass-shape dispatcher used by the next test: writes go straight
/// to disk. This mirrors the shape the runtime's
/// `BypassWriteDispatcher` will have without the runtime's
/// surrounding plumbing (event bus, store, etc.).
struct DiskBypassDispatcher;

#[async_trait]
impl WriteDispatcher for DiskBypassDispatcher {
    async fn workspace_write(&self, op: WorkspaceWrite) -> Result<Value, ToolError> {
        if let Some(parent) = op.abs.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&op.abs, op.after.as_bytes())
            .await
            .unwrap();
        Ok(json!({ "written": op.rel_path }))
    }

    async fn job_scope_write(&self, op: JobScopeWrite) -> Result<Value, ToolError> {
        // The special case: bypass must not let job-scope writes go
        // around `jobs.updateScope`. The dispatcher would normally
        // delegate to that RPC; the test stand-in records the call so
        // the assertion can verify the routing happened.
        Ok(json!({
            "routed": "jobs.updateScope",
            "segment": op.segment,
            "tail": op.tail,
        }))
    }
}

#[tokio::test]
async fn bypass_mode_writes_through_for_non_job_scope_path() {
    let root = TempDir::new().unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let mut registry = ToolRegistry::new();
    register_for_mode(
        &mut registry,
        Arc::clone(&sandbox),
        "bypass",
        Arc::new(DiskBypassDispatcher),
    );
    let harness = fake_ctx();
    registry
        .get("fs.write")
        .unwrap()
        .call(
            &harness.ctx,
            json!({ "path": "src/lib.rs", "content": "fn main() {}" }),
        )
        .await
        .unwrap();
    let on_disk = tokio::fs::read_to_string(root.path().join("src/lib.rs"))
        .await
        .unwrap();
    assert_eq!(on_disk, "fn main() {}");
}

#[tokio::test]
async fn job_scope_path_routes_through_jobs_update_scope_in_bypass() {
    // D3: the special case. Even in bypass, a write under
    // `.codeless/jobs/<name>/` must call the dispatcher's
    // `job_scope_write` method (which delegates to
    // `jobs.updateScope` in production). The paused-job rule is a
    // runtime invariant, not a permission; bypass cannot opt out.
    let root = TempDir::new().unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let rec = Arc::new(RecordingDispatcher::new());
    let mut registry = ToolRegistry::new();
    register_for_mode(&mut registry, Arc::clone(&sandbox), "bypass", rec.clone());
    let harness = fake_ctx();
    let out = registry
        .get("fs.write")
        .unwrap()
        .call(
            &harness.ctx,
            json!({
                "path": ".codeless/jobs/foo/SCOPE.md",
                "content": "# Scope\n",
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        out.get("routed").and_then(Value::as_str),
        Some("jobs.updateScope")
    );
    // Disk under `.codeless/jobs/` must stay untouched — the
    // dispatcher routes the write through `jobs.updateScope` rather
    // than writing the file path directly.
    assert!(
        tokio::fs::metadata(root.path().join(".codeless/jobs/foo/SCOPE.md"))
            .await
            .is_err(),
        "bypass write under .codeless/jobs/<name>/ must route through jobs.updateScope, \
         not write the file path directly",
    );
    let calls = rec.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        matches!(&calls[0], RecordedWrite::JobScope { segment, tail, .. }
        if segment == "foo" && tail == "SCOPE.md")
    );
}

#[tokio::test]
async fn job_scope_path_routes_through_jobs_update_scope_in_approve_edits() {
    // Same special case, approve-edits side. The Tool still hands
    // the write to the dispatcher via `job_scope_write`; whether the
    // dispatcher then surfaces a card or runs the RPC immediately is
    // the dispatcher's concern (the production approve-edits impl
    // creates an EditScope action card pointing at jobs.updateScope).
    let root = TempDir::new().unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let rec = Arc::new(RecordingDispatcher::new());
    let mut registry = ToolRegistry::new();
    register_for_mode(
        &mut registry,
        Arc::clone(&sandbox),
        "approve-edits",
        rec.clone(),
    );
    let harness = fake_ctx();
    registry
        .get("fs.write")
        .unwrap()
        .call(
            &harness.ctx,
            json!({
                "path": ".codeless/jobs/bar/WORKFLOW.md",
                "content": "x",
            }),
        )
        .await
        .unwrap();
    assert!(
        matches!(&rec.calls()[0], RecordedWrite::JobScope { segment, tail, .. }
        if segment == "bar" && tail == "WORKFLOW.md")
    );
}

#[tokio::test]
async fn registers_exactly_fs_write_and_fs_edit_when_mode_grants_writes() {
    let root = TempDir::new().unwrap();
    let sandbox = Arc::new(Sandbox::new(root.path()));
    let mut registry = ToolRegistry::new();
    register_for_mode(
        &mut registry,
        sandbox,
        "approve-edits",
        Arc::new(RecordingDispatcher::new()),
    );
    let mut names: Vec<&str> = registry.names().collect();
    names.sort();
    assert_eq!(names, vec!["fs.edit", "fs.write"]);
}
