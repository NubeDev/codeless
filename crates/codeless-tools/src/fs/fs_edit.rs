//! `fs.edit` — exact-string replace inside one workspace file.
//!
//! Unlike `fs.write`, the file must already exist and the `old`
//! string must occur exactly once. Ambiguity (zero or multiple
//! matches) returns a typed error rather than guessing; the planner
//! is meant to narrow the `old` snippet until it is unique.
//!
//! Per SCOPE-ASSISTANT-FS D7, the action card pre-renders the
//! post-replace content into `after` so the diff card shows the
//! literal future state rather than the `(old, new)` tuple. The
//! Tool computes the replaced body in memory and hands it to the
//! same [`WriteDispatcher`] surface `fs.write` uses; the dispatcher
//! does not need a second action variant.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

use super::dispatch::{classify_target, JobScopeWrite, WorkspaceWrite, WriteTarget};
use super::{Sandbox, SharedWriteDispatcher};

pub struct FsEditTool {
    schema: Value,
    sandbox: Arc<Sandbox>,
    dispatcher: SharedWriteDispatcher,
}

impl FsEditTool {
    pub fn new(sandbox: Arc<Sandbox>, dispatcher: SharedWriteDispatcher) -> Self {
        Self {
            sandbox,
            dispatcher,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path. File must already exist."
                    },
                    "old": {
                        "type": "string",
                        "description": "Exact substring to replace. Must occur exactly once in the file."
                    },
                    "new": {
                        "type": "string",
                        "description": "Replacement text."
                    }
                },
                "required": ["path", "old", "new"]
            }),
        }
    }
}

#[async_trait]
impl Tool for FsEditTool {
    fn name(&self) -> &str {
        "fs.edit"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'path'"))?;
        let old = args
            .get("old")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'old'"))?;
        let new_text = args
            .get("new")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'new'"))?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        if old.is_empty() {
            // An empty `old` would otherwise match at every byte
            // boundary; the only sensible interpretation is "create a
            // brand-new file", which is what `fs.write` is for.
            return Err(ToolError::invalid_args(
                "fs.edit `old` is empty; use fs.write to create or overwrite a file",
            ));
        }

        self.sandbox.check_relative_syntax(path)?;
        let target = classify_target(path).ok_or_else(|| {
            ToolError::invalid_args(format!(
                "path '{path}' does not name a writable workspace file"
            ))
        })?;

        // Both arms must read the file's current body to apply the
        // replace. Job-scope reads still go through the sandbox so a
        // symlink masquerading as `.codeless/jobs/foo/SCOPE.md` cannot
        // smuggle content from elsewhere; the canonical absolute path
        // is then discarded because the dispatcher writes back through
        // jobs.updateScope, not through that path.
        let rel_path = match &target {
            WriteTarget::Workspace { rel_path } => rel_path.clone(),
            WriteTarget::JobScope { rel_path, .. } => rel_path.clone(),
        };
        let abs = self.sandbox.resolve_existing(&rel_path).await?;
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|e| ToolError::failed(format!("read '{rel_path}': {e}")))?;
        let before = String::from_utf8(bytes).map_err(|_| {
            // An exact-string edit on a binary blob is not meaningful;
            // the planner should be using fs.write to replace it
            // wholesale or leaving it alone.
            ToolError::invalid_args(format!(
                "file '{rel_path}' is not valid UTF-8; fs.edit operates on text only"
            ))
        })?;

        let occurrences = before.matches(old).count();
        if occurrences == 0 {
            return Err(ToolError::failed(format!(
                "fs.edit could not find `old` in '{rel_path}'; narrow or correct the snippet"
            )));
        }
        if occurrences > 1 {
            return Err(ToolError::failed(format!(
                "fs.edit found `old` {occurrences} times in '{rel_path}'; \
                 widen the snippet to make it unique"
            )));
        }
        let after = before.replacen(old, new_text, 1);

        match target {
            WriteTarget::JobScope {
                rel_path,
                segment,
                tail,
            } => {
                self.dispatcher
                    .job_scope_write(JobScopeWrite {
                        rel_path,
                        segment,
                        tail,
                        after,
                    })
                    .await
            }
            WriteTarget::Workspace { rel_path } => {
                self.dispatcher
                    .workspace_write(WorkspaceWrite {
                        rel_path,
                        abs,
                        before: Some(before),
                        after,
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
    async fn pre_renders_after_content_into_dispatcher() {
        // D7: the card body is the literal post-replace content, not
        // the (old, new) tuple. The tool computes `after` in memory
        // and hands it to the dispatcher; a downstream renderer can
        // diff `before` vs `after` without re-running the replace.
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("notes.txt"), "alpha beta gamma")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let rec = Arc::new(RecordingDispatcher::new());
        let tool = FsEditTool::new(sandbox, rec.clone());
        tool.call(
            &h.ctx,
            json!({ "path": "notes.txt", "old": "beta", "new": "BETA" }),
        )
        .await
        .unwrap();
        match &rec.calls()[0] {
            RecordedWrite::Workspace { before, after, .. } => {
                assert_eq!(before.as_deref(), Some("alpha beta gamma"));
                assert_eq!(after, "alpha BETA gamma");
            }
            other => panic!("expected Workspace, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ambiguous_match_rejects_with_failed() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), "x x x")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(&h.ctx, json!({ "path": "a.txt", "old": "x", "new": "y" }))
            .await
            .unwrap_err();
        match err {
            ToolError::Failed(msg) => assert!(msg.contains("3 times"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_match_rejects_with_failed() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), "hello")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(
                &h.ctx,
                json!({ "path": "a.txt", "old": "world", "new": "x" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn empty_old_rejected_as_invalid_args() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), "x")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(&h.ctx, json!({ "path": "a.txt", "old": "", "new": "y" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn missing_file_rejects_via_sandbox() {
        // `fs.edit` resolves through `resolve_existing`; a missing
        // file is a Failed (typo, not policy) per the sandbox docs.
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(
                &h.ctx,
                json!({ "path": "nope.txt", "old": "a", "new": "b" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn job_scope_edit_routes_through_job_scope_dispatcher() {
        let root = TempDir::new().unwrap();
        tokio::fs::create_dir_all(root.path().join(".codeless/jobs/foo"))
            .await
            .unwrap();
        tokio::fs::write(
            root.path().join(".codeless/jobs/foo/SCOPE.md"),
            "version 1\n",
        )
        .await
        .unwrap();
        let (sandbox, h) = harness(&root);
        let rec = Arc::new(RecordingDispatcher::new());
        let tool = FsEditTool::new(sandbox, rec.clone());
        tool.call(
            &h.ctx,
            json!({
                "path": ".codeless/jobs/foo/SCOPE.md",
                "old": "version 1",
                "new": "version 2",
            }),
        )
        .await
        .unwrap();
        match &rec.calls()[0] {
            RecordedWrite::JobScope {
                segment,
                tail,
                after,
                ..
            } => {
                assert_eq!(segment, "foo");
                assert_eq!(tail, "SCOPE.md");
                assert_eq!(after, "version 2\n");
            }
            other => panic!("expected JobScope, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bypass_dispatcher_writes_replacement_to_disk() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), "alpha")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(FsBypassDispatcher));
        tool.call(
            &h.ctx,
            json!({ "path": "a.txt", "old": "alpha", "new": "omega" }),
        )
        .await
        .unwrap();
        let on_disk = tokio::fs::read_to_string(root.path().join("a.txt"))
            .await
            .unwrap();
        assert_eq!(on_disk, "omega");
    }

    #[tokio::test]
    async fn binary_file_rejects_invalid_args() {
        let root = TempDir::new().unwrap();
        // Invalid UTF-8 byte sequence.
        tokio::fs::write(root.path().join("blob.bin"), [0xff, 0xfe, 0xfd])
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsEditTool::new(sandbox, Arc::new(RecordingDispatcher::new()));
        let err = tool
            .call(
                &h.ctx,
                json!({ "path": "blob.bin", "old": "x", "new": "y" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }
}
