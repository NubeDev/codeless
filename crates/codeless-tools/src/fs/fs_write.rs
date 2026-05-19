//! `fs.write` — create or overwrite a workspace file.
//!
//! Only the sandbox check and the job-scope classifier live here; the
//! mutation itself is delegated to a [`WriteDispatcher`] so the same
//! tool body works for `approve-edits` (surface a card) and `bypass`
//! (write through) without the Tool impl learning the two
//! conditional flows (SCOPE-ASSISTANT-FS D1 / D3). `read-only` never
//! reaches this tool because the registration helper omits it
//! outright on that mode (D8).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

use super::dispatch::{classify_target, JobScopeWrite, WorkspaceWrite, WriteTarget};
use super::{Sandbox, SharedWriteDispatcher};

/// Hard byte cap for `fs.write` content. Mirrors `fs.read`'s
/// [`READ_BYTE_CAP`][super::READ_BYTE_CAP] so a planner reading a
/// file and writing it back through the same pair cannot trip a
/// cap mismatch. Larger writes return a typed `too_large` error.
pub const WRITE_BYTE_CAP: u64 = 5 * 1024 * 1024;

pub struct FsWriteTool {
    schema: Value,
    sandbox: Arc<Sandbox>,
    dispatcher: SharedWriteDispatcher,
    cap: u64,
}

impl FsWriteTool {
    pub fn new(sandbox: Arc<Sandbox>, dispatcher: SharedWriteDispatcher) -> Self {
        Self::with_cap(sandbox, dispatcher, WRITE_BYTE_CAP)
    }

    /// Cap override for tests; production code reaches for
    /// [`FsWriteTool::new`].
    pub fn with_cap(sandbox: Arc<Sandbox>, dispatcher: SharedWriteDispatcher, cap: u64) -> Self {
        Self {
            sandbox,
            dispatcher,
            cap,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path. Parent directories are created if missing."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full new file body. Existing files are overwritten."
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }
}

#[async_trait]
impl Tool for FsWriteTool {
    fn name(&self) -> &str {
        "fs.write"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'path'"))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'content'"))?
            .to_owned();
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let len = content.len() as u64;
        if len > self.cap {
            return Err(ToolError::failed(format!(
                "content for '{path}' is {len} bytes; exceeds {} byte cap",
                self.cap
            )));
        }
        // Syntax check upstream of the classifier so an absolute /
        // `..` path is rejected as Denied instead of being silently
        // normalised into a workspace-relative spelling by
        // [`classify_target`].
        self.sandbox.check_relative_syntax(path)?;
        let target = classify_target(path).ok_or_else(|| {
            ToolError::invalid_args(format!(
                "path '{path}' does not name a writable workspace file"
            ))
        })?;
        match target {
            WriteTarget::JobScope {
                rel_path,
                segment,
                tail,
            } => {
                // D3: the special case is unconditional. The dispatcher
                // routes through jobs.updateScope regardless of mode;
                // the paused-job guard lives in that RPC.
                self.dispatcher
                    .job_scope_write(JobScopeWrite {
                        rel_path,
                        segment,
                        tail,
                        after: content,
                    })
                    .await
            }
            WriteTarget::Workspace { rel_path } => {
                let abs = self.sandbox.resolve_for_create(&rel_path).await?;
                // Capture the existing content so the dispatcher can
                // surface a diff in the action-card path (D7). A
                // missing file is `None`, not an error — `fs.write`
                // is create-or-overwrite.
                let before = match tokio::fs::read(&abs).await {
                    Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        return Err(ToolError::failed(format!(
                            "stat-for-diff '{rel_path}' failed: {e}"
                        )));
                    }
                };
                self.dispatcher
                    .workspace_write(WorkspaceWrite {
                        rel_path,
                        abs,
                        before,
                        after: content,
                    })
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::dispatch::fs_bypass_for_tests::FsBypassDispatcher;
    use crate::fs::dispatch::test_helpers::{RecordedWrite, RecordingDispatcher};
    use crate::testing::fake_ctx_builder;
    use tempfile::TempDir;

    fn harness(root: &TempDir) -> (Arc<Sandbox>, crate::testing::FakeCtx) {
        (
            Arc::new(Sandbox::new(root.path())),
            fake_ctx_builder().build(),
        )
    }

    #[tokio::test]
    async fn workspace_write_passes_through_dispatcher() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let rec = Arc::new(RecordingDispatcher::new());
        let tool = FsWriteTool::new(sandbox, rec.clone());

        let out = tool
            .call(
                &h.ctx,
                json!({ "path": "src/lib.rs", "content": "fn main() {}" }),
            )
            .await
            .unwrap();
        assert_eq!(out.get("pending").and_then(Value::as_bool), Some(true));
        let calls = rec.calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            RecordedWrite::Workspace {
                rel_path,
                before,
                after,
                ..
            } => {
                assert_eq!(rel_path, "src/lib.rs");
                // Brand-new file: `before` is None — the dispatcher's
                // diff-render code falls back to "added the whole
                // body" when this is empty.
                assert!(before.is_none(), "new file has no before content");
                assert_eq!(after, "fn main() {}");
            }
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn job_scope_path_routes_through_job_scope_dispatcher() {
        // The special case (D3). A write under `.codeless/jobs/<seg>/`
        // must call `job_scope_write`, never `workspace_write`, no
        // matter which dispatcher is wired — the paused-job guard
        // applies in every mode including bypass.
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let rec = Arc::new(RecordingDispatcher::new());
        let tool = FsWriteTool::new(sandbox, rec.clone());

        let out = tool
            .call(
                &h.ctx,
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
        let calls = rec.calls();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            RecordedWrite::JobScope {
                segment,
                tail,
                after,
                ..
            } => {
                assert_eq!(segment, "foo");
                assert_eq!(tail, "SCOPE.md");
                assert_eq!(after, "# Scope\n");
            }
            other => panic!("expected JobScope, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bypass_dispatcher_writes_through_to_disk() {
        // The bypass shape: dispatcher actually creates the file.
        // Confirms the WriteOp's `abs` field lands a write at the
        // sandbox-canonicalised path, not the raw relative spelling.
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsWriteTool::new(sandbox, Arc::new(FsBypassDispatcher));

        tool.call(
            &h.ctx,
            json!({
                "path": "new/dir/hello.txt",
                "content": "hi",
            }),
        )
        .await
        .unwrap();

        let on_disk = tokio::fs::read_to_string(root.path().join("new/dir/hello.txt"))
            .await
            .unwrap();
        assert_eq!(on_disk, "hi");
    }

    #[tokio::test]
    async fn rejects_absolute_path_via_sandbox() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsWriteTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(&h.ctx, json!({ "path": "/etc/passwd", "content": "owned" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_parent_traversal_via_sandbox() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsWriteTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(&h.ctx, json!({ "path": "../escape", "content": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_oversize_content() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        // Tiny cap so we do not have to build a 5 MiB fixture.
        let tool = FsWriteTool::with_cap(sandbox, Arc::new(RecordingDispatcher::new()), 4);
        let err = tool
            .call(&h.ctx, json!({ "path": "a.txt", "content": "12345" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn overwriting_existing_file_carries_before_content() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), b"old body")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let rec = Arc::new(RecordingDispatcher::new());
        let tool = FsWriteTool::new(sandbox, rec.clone());
        tool.call(&h.ctx, json!({ "path": "a.txt", "content": "new body" }))
            .await
            .unwrap();
        match &rec.calls()[0] {
            RecordedWrite::Workspace { before, after, .. } => {
                assert_eq!(before.as_deref(), Some("old body"));
                assert_eq!(after, "new body");
            }
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refuses_job_scope_directory_target() {
        // `.codeless/jobs/foo` names a directory, not a file. The
        // classifier returns None and the tool surfaces InvalidArgs
        // rather than routing a directory write through
        // jobs.updateScope.
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsWriteTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(
                &h.ctx,
                json!({ "path": ".codeless/jobs/foo", "content": "x" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }
}
