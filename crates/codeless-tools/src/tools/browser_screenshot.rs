// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.screenshot. Two delivery shapes:
//   - default: base64-encoded image returned in-band by the sidecar
//   - save_to set: the sidecar writes the file directly and we
//     return the path. save_to is resolved against
//     ToolCtx::worktree_root and rejected if it escapes the
//     worktree (codeless-native replacement for moxxy's PathPolicy).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

pub struct BrowserScreenshotTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserScreenshotTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "full_page": { "type": "boolean" },
                    "format": { "type": "string", "enum": ["png", "jpeg"] },
                    "quality": { "type": "integer" },
                    "save_to": {
                        "type": "string",
                        "description": "Worktree-relative path to save into. Escape attempts (.. or absolute) are rejected."
                    },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "codeless.browser.screenshot"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if args.get("page_id").and_then(Value::as_str).is_none() {
            return Err(ToolError::invalid_args("missing 'page_id'"));
        }
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis);

        let mut sidecar_params = args.clone();
        if let Some(rel) = args.get("save_to").and_then(Value::as_str) {
            let resolved = resolve_worktree_path(ctx.worktree_root(), rel)?;
            if let Some(obj) = sidecar_params.as_object_mut() {
                obj.remove("save_to");
                obj.insert(
                    "save_to_path".to_string(),
                    Value::String(resolved.to_string_lossy().into_owned()),
                );
            }
        }
        self.manager
            .request("page.screenshot", sidecar_params, timeout)
            .await
    }
}

/// Resolve a worktree-relative path and reject anything that would
/// escape the worktree.
///
/// Codeless-native replacement for moxxy's `PathPolicy::resolve_path
/// + ensure_writable`. Rules:
///   - Absolute paths are rejected.
///   - `..` components are rejected at the policy layer rather than
///     after canonicalisation, so even non-existent parents fail
///     fast (the screenshot hasn't been written yet, so canonicalize
///     can't be used).
///   - The resolved path is `worktree_root.join(rel)` with no
///     symlink-following, on the assumption that worktrees are
///     newly-created and don't contain attacker-controlled symlinks.
fn resolve_worktree_path(worktree: &Path, rel: &str) -> Result<PathBuf, ToolError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(ToolError::invalid_args(format!(
            "save_to must be worktree-relative, got absolute path '{rel}'"
        )));
    }
    for c in rel_path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(ToolError::invalid_args(format!(
                    "save_to '{rel}' contains '..' — would escape the worktree"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::invalid_args(format!(
                    "save_to '{rel}' is not a plain relative path"
                )));
            }
        }
    }
    Ok(worktree.join(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_accepts_simple_relative_path() {
        let root = Path::new("/tmp/wt");
        let out = resolve_worktree_path(root, "out/img.png").unwrap();
        assert_eq!(out, PathBuf::from("/tmp/wt/out/img.png"));
    }

    #[test]
    fn resolve_rejects_absolute() {
        let err = resolve_worktree_path(Path::new("/tmp/wt"), "/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn resolve_rejects_parent_traversal() {
        let err = resolve_worktree_path(Path::new("/tmp/wt"), "../escape").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn resolve_rejects_mid_path_parent_traversal() {
        let err = resolve_worktree_path(Path::new("/tmp/wt"), "ok/../../escape").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn resolve_allows_cur_dir() {
        let out = resolve_worktree_path(Path::new("/tmp/wt"), "./out.png").unwrap();
        assert_eq!(out, PathBuf::from("/tmp/wt/./out.png"));
    }
}
