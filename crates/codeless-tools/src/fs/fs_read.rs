//! `fs.read` — workspace-relative file read.
//!
//! 5 MiB hard cap (SCOPE-ASSISTANT-FS D5). Larger files return a
//! typed `too_large` error rather than a partial read; the planner
//! is meant to narrow the request (line range, `fs.search`) instead
//! of paging through a blob. Binary content is returned as a
//! lossy-UTF-8 string with the original byte length so the model
//! still sees the size signal even when the content is not text.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

use super::Sandbox;

/// Hard byte cap for `fs.read` (SCOPE-ASSISTANT-FS D5). 5 MiB sits
/// well below typical planner context budgets while admitting every
/// scope / template / source file the assistant is likely to see.
pub const READ_BYTE_CAP: u64 = 5 * 1024 * 1024;

pub struct FsReadTool {
    schema: Value,
    sandbox: Arc<Sandbox>,
    cap: u64,
}

impl FsReadTool {
    pub fn new(sandbox: Arc<Sandbox>) -> Self {
        Self::with_cap(sandbox, READ_BYTE_CAP)
    }

    /// Cap override for tests. Production code reaches for
    /// [`FsReadTool::new`]; tests use this to exercise the cap path
    /// without writing a 5 MiB fixture.
    pub fn with_cap(sandbox: Arc<Sandbox>, cap: u64) -> Self {
        Self {
            sandbox,
            cap,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative file path."
                    }
                },
                "required": ["path"]
            }),
        }
    }
}

#[async_trait]
impl Tool for FsReadTool {
    fn name(&self) -> &str {
        "fs.read"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'path'"))?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let target = self.sandbox.resolve_existing(path).await?;
        let meta = tokio::fs::metadata(&target)
            .await
            .map_err(|e| ToolError::failed(format!("stat '{path}' failed: {e}")))?;
        if meta.is_dir() {
            return Err(ToolError::invalid_args(format!(
                "path '{path}' is a directory; use fs.list"
            )));
        }
        let size = meta.len();
        if size > self.cap {
            // The error message names the cap so the planner can
            // budget a narrower request without re-deriving the
            // constant from the doc.
            return Err(ToolError::failed(format!(
                "file '{path}' is {size} bytes; exceeds {} byte cap (narrow the request: \
                 line range or fs.search instead)",
                self.cap
            )));
        }
        let bytes = tokio::fs::read(&target)
            .await
            .map_err(|e| ToolError::failed(format!("read '{path}' failed: {e}")))?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        Ok(json!({
            "path": path,
            "size": size,
            "content": content,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake_ctx_builder;
    use tempfile::TempDir;

    fn harness(root: &TempDir) -> (Arc<Sandbox>, crate::testing::FakeCtx) {
        (
            Arc::new(Sandbox::new(root.path())),
            fake_ctx_builder().build(),
        )
    }

    #[tokio::test]
    async fn returns_file_content_and_size() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), b"hello")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsReadTool::new(sandbox);
        let out = tool.call(&h.ctx, json!({ "path": "a.txt" })).await.unwrap();
        assert_eq!(out.get("content").and_then(Value::as_str), Some("hello"));
        assert_eq!(out.get("size").and_then(Value::as_u64), Some(5));
    }

    #[tokio::test]
    async fn rejects_directory_with_invalid_args() {
        let root = TempDir::new().unwrap();
        tokio::fs::create_dir(root.path().join("d")).await.unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsReadTool::new(sandbox);
        let err = tool.call(&h.ctx, json!({ "path": "d" })).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn enforces_byte_cap() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("big.bin"), vec![0u8; 32])
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        // Cap below the file's actual size — the failure surface
        // we want, without writing a real 5 MiB fixture.
        let tool = FsReadTool::with_cap(sandbox, 16);
        let err = tool
            .call(&h.ctx, json!({ "path": "big.bin" }))
            .await
            .unwrap_err();
        match err {
            ToolError::Failed(msg) => assert!(msg.contains("exceeds"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_absolute_and_parent_paths() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsReadTool::new(sandbox);
        for bad in ["/etc/passwd", "../outside", "subdir/../../escape"] {
            let err = tool.call(&h.ctx, json!({ "path": bad })).await.unwrap_err();
            assert!(
                matches!(err, ToolError::Denied(_)),
                "expected Denied for {bad}, got {err:?}",
            );
        }
    }
}
