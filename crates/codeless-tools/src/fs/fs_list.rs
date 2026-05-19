//! `fs.list` — non-recursive directory listing.
//!
//! Returns each immediate child with its kind (`file` / `dir` /
//! `symlink`) and size for regular files. Hidden entries (leading
//! dot) and ignored entries (`.gitignore`) are listed; the planner
//! is read-only here and surfacing them is strictly more useful than
//! silently filtering them. Entries are sorted by name so a search
//! followed by a directory listing has a stable surface for the
//! model to reason about.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

use super::Sandbox;

pub struct FsListTool {
    schema: Value,
    sandbox: Arc<Sandbox>,
}

impl FsListTool {
    pub fn new(sandbox: Arc<Sandbox>) -> Self {
        Self {
            sandbox,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path. Use \".\" for the workspace root."
                    }
                },
                "required": ["path"]
            }),
        }
    }
}

#[async_trait]
impl Tool for FsListTool {
    fn name(&self) -> &str {
        "fs.list"
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
        // `.` is the conventional way to ask for the workspace root;
        // canonicalisation collapses it but a syntactic special-case
        // makes the schema description honest.
        let target = if path == "." {
            self.sandbox.canonical_root().await?
        } else {
            self.sandbox.resolve_existing(path).await?
        };

        let meta = tokio::fs::metadata(&target)
            .await
            .map_err(|e| ToolError::failed(format!("stat '{path}' failed: {e}")))?;
        if !meta.is_dir() {
            return Err(ToolError::invalid_args(format!(
                "path '{path}' is not a directory"
            )));
        }

        let mut dir = tokio::fs::read_dir(&target)
            .await
            .map_err(|e| ToolError::failed(format!("read_dir '{path}' failed: {e}")))?;
        let mut entries: Vec<Value> = Vec::new();
        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|e| ToolError::failed(format!("read_dir '{path}' failed: {e}")))?
        {
            // `file_type` is preferred over `metadata` because it
            // does not follow symlinks — a symlink shows as
            // `symlink`, and only `fs.read` (which calls
            // `Sandbox::resolve_existing`) follows it and revalidates
            // the target against the root.
            let ft = entry.file_type().await.map_err(|e| {
                ToolError::failed(format!("stat '{}' failed: {e}", entry.path().display()))
            })?;
            let kind = if ft.is_dir() {
                "dir"
            } else if ft.is_symlink() {
                "symlink"
            } else if ft.is_file() {
                "file"
            } else {
                "other"
            };
            let mut row = serde_json::Map::new();
            row.insert(
                "name".into(),
                Value::String(entry.file_name().to_string_lossy().into_owned()),
            );
            row.insert("kind".into(), Value::String(kind.into()));
            if ft.is_file() {
                if let Ok(m) = entry.metadata().await {
                    row.insert("size".into(), Value::from(m.len()));
                }
            }
            entries.push(Value::Object(row));
        }
        entries.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
        });
        Ok(json!({ "path": path, "entries": entries }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake_ctx_builder;
    use tempfile::TempDir;

    fn ctx_with_root(root: &TempDir) -> (Arc<Sandbox>, crate::testing::FakeCtx) {
        let sandbox = Arc::new(Sandbox::new(root.path()));
        let harness = fake_ctx_builder().build();
        (sandbox, harness)
    }

    #[tokio::test]
    async fn lists_directory_entries_sorted_by_name() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("b.txt"), b"bb")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("a.txt"), b"a")
            .await
            .unwrap();
        tokio::fs::create_dir(root.path().join("d")).await.unwrap();

        let (sandbox, harness) = ctx_with_root(&root);
        let tool = FsListTool::new(sandbox);
        let out = tool
            .call(&harness.ctx, json!({ "path": "." }))
            .await
            .unwrap();
        let entries = out.get("entries").and_then(Value::as_array).unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|e| e.get("name").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "d"]);
        // Regular files carry size; directories do not.
        let a = &entries[0];
        assert_eq!(a.get("kind").and_then(Value::as_str), Some("file"));
        assert_eq!(a.get("size").and_then(Value::as_u64), Some(1));
        let d = &entries[2];
        assert_eq!(d.get("kind").and_then(Value::as_str), Some("dir"));
        assert!(d.get("size").is_none());
    }

    #[tokio::test]
    async fn absolute_path_is_rejected() {
        let root = TempDir::new().unwrap();
        let (sandbox, harness) = ctx_with_root(&root);
        let tool = FsListTool::new(sandbox);
        let err = tool
            .call(&harness.ctx, json!({ "path": "/etc" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn parent_traversal_is_rejected() {
        let root = TempDir::new().unwrap();
        let (sandbox, harness) = ctx_with_root(&root);
        let tool = FsListTool::new(sandbox);
        let err = tool
            .call(&harness.ctx, json!({ "path": ".." }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn not_a_directory_is_invalid_args() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("f"), b"x").await.unwrap();
        let (sandbox, harness) = ctx_with_root(&root);
        let tool = FsListTool::new(sandbox);
        let err = tool
            .call(&harness.ctx, json!({ "path": "f" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn missing_path_arg_is_invalid_args() {
        let root = TempDir::new().unwrap();
        let (sandbox, harness) = ctx_with_root(&root);
        let tool = FsListTool::new(sandbox);
        let err = tool.call(&harness.ctx, json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }
}
