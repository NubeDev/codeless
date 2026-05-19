//! Workspace-root path sandbox for the assistant fs tools.
//!
//! Two layers of defence: a syntactic reject of absolute paths and
//! `..` components before any I/O, then a canonicalising re-check
//! against the canonical workspace root so a symlink whose target
//! escapes the root is rejected the same way as a literal traversal.
//! The canonicalising re-check is the load-bearing layer — the
//! syntactic check exists so a malformed input fails fast and with
//! a clearer error than `canonicalize` would surface.

use std::path::{Component, Path, PathBuf};

use crate::error::ToolError;

/// Workspace-root-bound path resolver. Constructed once per
/// assistant-thread context; tools call [`Sandbox::resolve_existing`]
/// on every dispatch. The root itself is not canonicalised eagerly —
/// the workspace may not exist on disk at construction time (a fresh
/// thread before its first scope write), and the per-call
/// canonicalisation surfaces a "workspace missing" error inline with
/// the same shape as any other I/O failure.
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `rel` (a workspace-relative path) to a canonical
    /// absolute path that is provably inside the workspace root.
    /// The target must already exist; a missing path is surfaced as
    /// [`ToolError::Failed`] so the planner can distinguish "I asked
    /// for the wrong thing" from "you are not allowed to ask for it"
    /// ([`ToolError::Denied`]).
    pub async fn resolve_existing(&self, rel: &str) -> Result<PathBuf, ToolError> {
        let parsed = check_relative(rel)?;
        let root = canonical_root(&self.root).await?;
        let joined = root.join(parsed);
        let canon = tokio::fs::canonicalize(&joined).await.map_err(|e| {
            ToolError::failed(format!(
                "path '{rel}' could not be resolved under the workspace root: {e}"
            ))
        })?;
        if !canon.starts_with(&root) {
            return Err(ToolError::denied(format!(
                "path '{rel}' resolves outside the workspace root"
            )));
        }
        Ok(canon)
    }

    /// Canonical workspace root. Exposed so tools that walk the
    /// whole tree (e.g. `fs.search` without an explicit `path`) can
    /// reuse the same canonicalisation rather than re-running it.
    pub async fn canonical_root(&self) -> Result<PathBuf, ToolError> {
        canonical_root(&self.root).await
    }
}

async fn canonical_root(root: &Path) -> Result<PathBuf, ToolError> {
    tokio::fs::canonicalize(root)
        .await
        .map_err(|e| ToolError::failed(format!("workspace root unavailable: {e}")))
}

/// Syntactic guard against absolute paths, drive-letter prefixes,
/// and `..` traversal. The error variants are picked so the
/// planner-facing distinction holds: a `Denied` is "policy refused
/// the path"; an `InvalidArgs` is "the path string itself is
/// malformed" (today only the empty string).
fn check_relative(rel: &str) -> Result<&Path, ToolError> {
    if rel.is_empty() {
        return Err(ToolError::invalid_args("path is empty"));
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(ToolError::denied(format!(
            "path '{rel}' is absolute; only workspace-relative paths are allowed"
        )));
    }
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                return Err(ToolError::denied(format!(
                    "path '{rel}' contains '..'; traversal outside the workspace is not allowed"
                )));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ToolError::denied(format!(
                    "path '{rel}' has a rooted prefix; only relative paths are allowed"
                )));
            }
            _ => {}
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sandbox_in(dir: &TempDir) -> Sandbox {
        Sandbox::new(dir.path())
    }

    #[tokio::test]
    async fn resolve_existing_returns_canonical_path_inside_root() {
        let tmp = TempDir::new().unwrap();
        tokio::fs::write(tmp.path().join("a.txt"), b"hi")
            .await
            .unwrap();
        let sandbox = sandbox_in(&tmp);
        let resolved = sandbox.resolve_existing("a.txt").await.unwrap();
        let root = tokio::fs::canonicalize(tmp.path()).await.unwrap();
        assert!(resolved.starts_with(&root));
        assert_eq!(resolved.file_name().unwrap(), "a.txt");
    }

    #[tokio::test]
    async fn absolute_path_is_denied() {
        let tmp = TempDir::new().unwrap();
        let sandbox = sandbox_in(&tmp);
        let err = sandbox.resolve_existing("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn parent_dir_component_is_denied() {
        let tmp = TempDir::new().unwrap();
        let sandbox = sandbox_in(&tmp);
        let err = sandbox.resolve_existing("../escape").await.unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
        // Embedded `..` is caught too — not just leading.
        let err = sandbox
            .resolve_existing("subdir/../../escape")
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn empty_path_is_invalid_args() {
        let tmp = TempDir::new().unwrap();
        let sandbox = sandbox_in(&tmp);
        let err = sandbox.resolve_existing("").await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn missing_path_is_failure_not_denial() {
        // The distinction matters for the planner: a typo'd filename
        // is `Failed`; an out-of-root path is `Denied`. Conflating
        // them would make policy violations look like recoverable
        // mistakes.
        let tmp = TempDir::new().unwrap();
        let sandbox = sandbox_in(&tmp);
        let err = sandbox.resolve_existing("nope.txt").await.unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)), "got {err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_pointing_outside_root_is_denied() {
        let outer = TempDir::new().unwrap();
        let target = outer.path().join("secret.txt");
        tokio::fs::write(&target, b"out-of-bounds").await.unwrap();

        let root = TempDir::new().unwrap();
        let link = root.path().join("link");
        tokio::fs::symlink(&target, &link).await.unwrap();

        let sandbox = sandbox_in(&root);
        let err = sandbox.resolve_existing("link").await.unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn nested_relative_path_resolves() {
        let tmp = TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("d")).await.unwrap();
        tokio::fs::write(tmp.path().join("d/b.txt"), b"x")
            .await
            .unwrap();
        let sandbox = sandbox_in(&tmp);
        let resolved = sandbox.resolve_existing("d/b.txt").await.unwrap();
        assert!(resolved.ends_with("d/b.txt"));
    }
}
