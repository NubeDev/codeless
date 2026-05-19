//! `fs.search` — ripgrep-style content search across the workspace.
//!
//! Pure-Rust by R1: spawning `rg` would force the tool into
//! `codeless-adapters-host` and add a binary dependency users may
//! not have installed. Traversal uses `ignore::Walk` so `.gitignore`
//! and `.ignore` are honoured by default; matching is a regex per
//! line (case-sensitive, multi-byte safe via `regex`).
//!
//! The result cap (200 matches; SCOPE-ASSISTANT-FS D6) is set so
//! the assistant cannot eat its own context window with one search.
//! A truncated result carries the `truncated` flag plus
//! `total_seen` so the planner sees the under-count and can narrow
//! (tighter glob, more specific query) rather than pretending the
//! 200th match was the last one.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

use super::Sandbox;

/// Hard cap on `fs.search` matches (SCOPE-ASSISTANT-FS D6). 200
/// matches at typical line lengths fits inside the planner's
/// context budget with room for the rest of the conversation.
pub const SEARCH_MATCH_CAP: usize = 200;

/// Per-file byte cap. Searching a 100 MiB blob line-by-line would
/// stall the planner; binaries and oversized lockfiles are skipped
/// rather than partially read. The cap is generous enough that
/// almost every source file in a normal repo is searched in full.
const SEARCH_FILE_BYTE_CAP: u64 = 2 * 1024 * 1024;

pub struct FsSearchTool {
    schema: Value,
    sandbox: Arc<Sandbox>,
    cap: usize,
}

impl FsSearchTool {
    pub fn new(sandbox: Arc<Sandbox>) -> Self {
        Self::with_cap(sandbox, SEARCH_MATCH_CAP)
    }

    pub fn with_cap(sandbox: Arc<Sandbox>, cap: usize) -> Self {
        Self {
            sandbox,
            cap,
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Regex pattern, evaluated per line. Anchor with ^/$ as needed."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional glob restricting which files are searched, e.g. \"**/*.rs\"."
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional workspace-relative subdirectory to search under. Defaults to the workspace root."
                    }
                },
                "required": ["query"]
            }),
        }
    }
}

#[async_trait]
impl Tool for FsSearchTool {
    fn name(&self) -> &str {
        "fs.search"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'query'"))?
            .to_owned();
        let glob = args
            .get("glob")
            .and_then(Value::as_str)
            .map(|s| s.to_owned());
        let path_arg = args
            .get("path")
            .and_then(Value::as_str)
            .map(|s| s.to_owned());

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let re = Regex::new(&query)
            .map_err(|e| ToolError::invalid_args(format!("invalid query regex: {e}")))?;

        // The walk needs an absolute, canonical root to keep the
        // emitted paths workspace-relative. Resolving `path` here
        // means the sandbox check fires before the blocking walker
        // is spawned.
        let walk_root = if let Some(p) = path_arg.as_deref() {
            self.sandbox.resolve_existing(p).await?
        } else {
            self.sandbox.canonical_root().await?
        };
        let workspace_root = self.sandbox.canonical_root().await?;
        let cap = self.cap;
        let cancel = ctx.cancel_token().clone();

        // `ignore::Walk` is synchronous; spawning on the blocking
        // pool keeps the async runtime responsive while a large
        // workspace is traversed.
        let outcome = tokio::task::spawn_blocking(move || {
            walk_and_match(
                &workspace_root,
                &walk_root,
                &re,
                glob.as_deref(),
                cap,
                &cancel,
            )
        })
        .await
        .map_err(|e| ToolError::failed(format!("search task join failed: {e}")))??;

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        Ok(json!({
            "matches": outcome.matches,
            "truncated": outcome.truncated,
            "total_seen": outcome.total_seen,
        }))
    }
}

struct SearchOutcome {
    matches: Vec<Value>,
    truncated: bool,
    /// Total matches seen including those past the cap. The planner
    /// needs the real count to decide whether to narrow rather than
    /// trusting the truncated suffix.
    total_seen: u64,
}

fn walk_and_match(
    workspace_root: &PathBuf,
    walk_root: &PathBuf,
    re: &Regex,
    glob: Option<&str>,
    cap: usize,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<SearchOutcome, ToolError> {
    let mut builder = WalkBuilder::new(walk_root);
    builder.follow_links(false);
    if let Some(pattern) = glob {
        let mut ov = OverrideBuilder::new(walk_root);
        ov.add(pattern)
            .map_err(|e| ToolError::invalid_args(format!("invalid glob '{pattern}': {e}")))?;
        let ov = ov
            .build()
            .map_err(|e| ToolError::invalid_args(format!("invalid glob '{pattern}': {e}")))?;
        builder.overrides(ov);
    }

    let mut matches: Vec<Value> = Vec::new();
    let mut total_seen: u64 = 0;

    for result in builder.build() {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let abs = entry.path();
        // `ignore::Walk` can be asked to start at a subdir; the
        // canonical workspace root is what we surface in results so
        // the planner sees workspace-relative paths regardless of
        // which `path` arg the caller passed.
        let rel = abs.strip_prefix(workspace_root).unwrap_or(abs);
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.len() > SEARCH_FILE_BYTE_CAP {
            continue;
        }
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Skip files that don't look like text. A NUL byte is the
        // cheap signal `ripgrep` itself uses; surfaces of garbled
        // matches inside a PDF or PNG would burn context with no
        // upside.
        if bytes.contains(&0u8) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            if !re.is_match(line) {
                continue;
            }
            total_seen += 1;
            if matches.len() < cap {
                matches.push(json!({
                    "path": rel.to_string_lossy(),
                    "line": (i as u64) + 1,
                    "text": line,
                }));
            }
        }
    }
    let truncated = (matches.len() as u64) < total_seen;
    Ok(SearchOutcome {
        matches,
        truncated,
        total_seen,
    })
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
    async fn finds_matches_with_line_numbers() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.txt"), "alpha\nbeta\ngamma\n")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("b.txt"), "beta only\n")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsSearchTool::new(sandbox);
        let out = tool.call(&h.ctx, json!({ "query": "beta" })).await.unwrap();
        let matches = out.get("matches").and_then(Value::as_array).unwrap();
        assert_eq!(matches.len(), 2);
        // The line in `a.txt` is line 2; the line in `b.txt` is line 1.
        let by_path: std::collections::HashMap<&str, u64> = matches
            .iter()
            .map(|m| {
                (
                    m.get("path").and_then(Value::as_str).unwrap(),
                    m.get("line").and_then(Value::as_u64).unwrap(),
                )
            })
            .collect();
        assert_eq!(by_path.get("a.txt"), Some(&2));
        assert_eq!(by_path.get("b.txt"), Some(&1));
        assert_eq!(out.get("truncated").and_then(Value::as_bool), Some(false));
    }

    #[tokio::test]
    async fn glob_filters_files() {
        let root = TempDir::new().unwrap();
        tokio::fs::write(root.path().join("a.rs"), "fn beta() {}\n")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("a.txt"), "beta\n")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsSearchTool::new(sandbox);
        let out = tool
            .call(&h.ctx, json!({ "query": "beta", "glob": "*.rs" }))
            .await
            .unwrap();
        let matches = out.get("matches").and_then(Value::as_array).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].get("path").and_then(Value::as_str), Some("a.rs"),);
    }

    #[tokio::test]
    async fn truncation_reports_total_seen() {
        let root = TempDir::new().unwrap();
        let mut body = String::new();
        for _ in 0..10 {
            body.push_str("hit me\n");
        }
        tokio::fs::write(root.path().join("a.txt"), body)
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        // Cap of 3 forces 7 over-the-cap entries.
        let tool = FsSearchTool::with_cap(sandbox, 3);
        let out = tool.call(&h.ctx, json!({ "query": "hit" })).await.unwrap();
        let matches = out.get("matches").and_then(Value::as_array).unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(out.get("truncated").and_then(Value::as_bool), Some(true));
        assert_eq!(out.get("total_seen").and_then(Value::as_u64), Some(10));
    }

    #[tokio::test]
    async fn invalid_regex_is_invalid_args() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsSearchTool::new(sandbox);
        let err = tool
            .call(&h.ctx, json!({ "query": "(unclosed" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_absolute_path_arg() {
        let root = TempDir::new().unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsSearchTool::new(sandbox);
        let err = tool
            .call(&h.ctx, json!({ "query": "beta", "path": "/etc" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn skips_binary_files() {
        let root = TempDir::new().unwrap();
        // NUL byte signals "binary"; the walker skips the file
        // without reporting a match even though the substring is
        // textually present.
        tokio::fs::write(root.path().join("blob.bin"), b"alpha\0beta")
            .await
            .unwrap();
        tokio::fs::write(root.path().join("a.txt"), "alpha\n")
            .await
            .unwrap();
        let (sandbox, h) = harness(&root);
        let tool = FsSearchTool::new(sandbox);
        let out = tool
            .call(&h.ctx, json!({ "query": "alpha" }))
            .await
            .unwrap();
        let matches = out.get("matches").and_then(Value::as_array).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].get("path").and_then(Value::as_str),
            Some("a.txt"),
        );
    }
}
