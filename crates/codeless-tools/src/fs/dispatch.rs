//! Mode-aware write dispatcher for the assistant write tools.
//!
//! `fs.write` and `fs.edit` resolve and validate the workspace-relative
//! path themselves, then hand the actual mutation off to a
//! [`WriteDispatcher`]. The dispatcher carries the per-thread mode
//! (SCOPE-ASSISTANT-FS D1) and the seam through which a write either
//! lands on disk (`bypass`) or surfaces as an
//! `AssistantActionCard` the user confirms via the existing
//! `confirm_assistant_action` dispatcher (`approve-edits`).
//!
//! The trait lives in `codeless-tools` so the Tool impls have a typed
//! contract without depending on the runtime; concrete impls live in
//! `codeless-runtime` where they can reach the store and the
//! `update_job_scope` RPC. `read-only` does not appear here — D8
//! omits the tools from the registry entirely on a read-only thread,
//! so this surface is never reached in that mode.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;

/// Workspace-relative segment that triggers the
/// [`jobs.updateScope`][SCOPE-ASSISTANT-FS D3] routing. The check
/// applies in every mode — the paused-job rule is a runtime invariant,
/// not a permission, so `bypass` cannot opt out of it.
pub const JOB_SCOPE_ROOT: &str = ".codeless/jobs";

/// One write the dispatcher receives, fully resolved against the
/// sandbox. `rel_path` is the workspace-relative spelling the planner
/// asked for (kept so dispatcher errors can echo the original); `abs`
/// is the canonical absolute form the sandbox produced and the
/// dispatcher writes to in `bypass`. `before` is `Some` when the
/// write replaces an existing file — `fs.edit` always populates it;
/// `fs.write` populates it when the file existed before the call.
#[derive(Debug)]
pub struct WorkspaceWrite {
    pub rel_path: String,
    pub abs: PathBuf,
    pub before: Option<String>,
    pub after: String,
}

/// One write that targets a `.codeless/jobs/<segment>/<tail>` path
/// (SCOPE-ASSISTANT-FS D3). `segment` is the job-name folder under
/// `.codeless/jobs/`; `tail` is the relative path inside that folder
/// (e.g. `SCOPE.md`, `WORKFLOW.md`). The dispatcher routes this
/// through `jobs.updateScope` in every mode — the constructor of the
/// outcome carries no choice.
#[derive(Debug)]
pub struct JobScopeWrite {
    pub rel_path: String,
    pub segment: String,
    pub tail: String,
    pub after: String,
}

/// Mode-aware write seam. `read-only` threads never instantiate one
/// of these because the registration helper omits the write tools
/// outright on that mode (D8); `approve-edits` and `bypass` each
/// supply an impl that decides whether the write lands now or is
/// staged as an action card.
#[async_trait]
pub trait WriteDispatcher: Send + Sync {
    /// Dispatch a write that targets a regular workspace file.
    async fn workspace_write(&self, op: WorkspaceWrite) -> Result<Value, ToolError>;

    /// Dispatch a write that targets a `.codeless/jobs/<name>/<tail>`
    /// path. Implementations route this through `jobs.updateScope`
    /// (and inherit its paused-job guard) regardless of the
    /// surrounding mode — bypass does not opt out.
    async fn job_scope_write(&self, op: JobScopeWrite) -> Result<Value, ToolError>;
}

/// Boxed type alias the Tool impls hold. `Arc` so a single dispatcher
/// instance can back the `fs.write` and `fs.edit` registrations
/// without each tool owning its own copy.
pub type SharedWriteDispatcher = Arc<dyn WriteDispatcher>;

/// Where a resolved workspace-relative path lands. Returned by
/// [`classify_target`] so the Tool impls can pick which dispatcher
/// method to call without re-walking the path twice. The split is
/// computed off the raw relative spelling, not the canonical absolute
/// form, so a planner naming `./.codeless/jobs/foo/x` routes the same
/// way as `.codeless/jobs/foo/x`.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteTarget {
    /// Regular workspace file. `rel_path` is the normalised
    /// workspace-relative spelling (leading `./` removed, components
    /// preserved otherwise).
    Workspace { rel_path: String },
    /// `.codeless/jobs/<segment>/<tail>` write. `segment` is the
    /// immediate folder under `.codeless/jobs/`; `tail` is the
    /// remainder, joined by `/`. Empty `tail` (i.e. the segment
    /// itself) is rejected by the caller because a write must name a
    /// file, not a directory.
    JobScope {
        rel_path: String,
        segment: String,
        tail: String,
    },
}

/// Classify a workspace-relative path. The sandbox's syntactic guard
/// already rejected absolute / `..` paths upstream; this helper only
/// folds away `./` segments and splits on the job-scope prefix. An
/// empty `tail` (path == `.codeless/jobs/<segment>`) is surfaced as
/// `None` so the caller can refuse the write with a typed error
/// rather than the dispatcher inheriting a "write to a directory"
/// failure.
pub fn classify_target(rel: &str) -> Option<WriteTarget> {
    let normalised = normalise_relative(rel);
    if normalised.is_empty() {
        return None;
    }
    let parts: Vec<&str> = normalised.split('/').collect();
    if parts.len() >= 3 && parts[0] == ".codeless" && parts[1] == "jobs" {
        let segment = parts[2].to_owned();
        if segment.is_empty() {
            return None;
        }
        let tail_parts = &parts[3..];
        if tail_parts.is_empty() {
            return None;
        }
        let tail = tail_parts.join("/");
        if tail.is_empty() {
            return None;
        }
        Some(WriteTarget::JobScope {
            rel_path: normalised,
            segment,
            tail,
        })
    } else {
        Some(WriteTarget::Workspace {
            rel_path: normalised,
        })
    }
}

fn normalise_relative(rel: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for comp in rel.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        out.push(comp);
    }
    out.join("/")
}

/// Test helpers — kept public so integration tests in this crate and
/// downstream crates can drive the Tool impls with a synthetic
/// dispatcher that records every call without performing any I/O.
/// The companion on-disk dispatcher (`fs_bypass_for_tests`) is gated
/// on `cfg(test)` because downstream crates use the concrete
/// dispatcher impls in `codeless-runtime` instead.
pub mod test_helpers {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    pub enum RecordedWrite {
        Workspace {
            rel_path: String,
            abs: PathBuf,
            before: Option<String>,
            after: String,
        },
        JobScope {
            rel_path: String,
            segment: String,
            tail: String,
            after: String,
        },
    }

    #[derive(Default)]
    pub struct RecordingDispatcher {
        calls: Mutex<Vec<RecordedWrite>>,
    }

    impl RecordingDispatcher {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn calls(&self) -> Vec<RecordedWrite> {
            self.calls.lock().unwrap().clone()
        }
        pub fn len(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
        pub fn is_empty(&self) -> bool {
            self.calls.lock().unwrap().is_empty()
        }
    }

    #[async_trait]
    impl WriteDispatcher for RecordingDispatcher {
        async fn workspace_write(&self, op: WorkspaceWrite) -> Result<Value, ToolError> {
            self.calls.lock().unwrap().push(RecordedWrite::Workspace {
                rel_path: op.rel_path.clone(),
                abs: op.abs.clone(),
                before: op.before.clone(),
                after: op.after.clone(),
            });
            Ok(serde_json::json!({ "pending": true, "path": op.rel_path }))
        }

        async fn job_scope_write(&self, op: JobScopeWrite) -> Result<Value, ToolError> {
            self.calls.lock().unwrap().push(RecordedWrite::JobScope {
                rel_path: op.rel_path.clone(),
                segment: op.segment.clone(),
                tail: op.tail.clone(),
                after: op.after.clone(),
            });
            Ok(serde_json::json!({
                "routed": "jobs.updateScope",
                "segment": op.segment,
                "tail": op.tail,
            }))
        }
    }
}

/// Companion to [`test_helpers`]: a trivial dispatcher
/// that actually performs the workspace write on disk (the `bypass`
/// shape, without the runtime). Job-scope writes are reported through
/// the second method so a test can assert the routing without standing
/// up `jobs.updateScope`. Kept gated on `cfg(test)` because it only
/// makes sense inside this crate's test runs — downstream crates use
/// the concrete dispatcher impls in `codeless-runtime`.
#[cfg(test)]
pub(crate) mod fs_bypass_for_tests {
    use super::*;
    use std::path::Path;

    pub struct FsBypassDispatcher;

    #[async_trait]
    impl WriteDispatcher for FsBypassDispatcher {
        async fn workspace_write(&self, op: WorkspaceWrite) -> Result<Value, ToolError> {
            if let Some(parent) = Path::new(&op.abs).parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    ToolError::failed(format!("create parents for '{}': {e}", op.rel_path))
                })?;
            }
            tokio::fs::write(&op.abs, op.after.as_bytes())
                .await
                .map_err(|e| ToolError::failed(format!("write '{}': {e}", op.rel_path)))?;
            Ok(serde_json::json!({
                "written": op.rel_path,
                "bytes": op.after.len(),
            }))
        }

        async fn job_scope_write(&self, op: JobScopeWrite) -> Result<Value, ToolError> {
            Ok(serde_json::json!({
                "routed": "jobs.updateScope",
                "segment": op.segment,
                "tail": op.tail,
                "bytes": op.after.len(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_normal_workspace_path() {
        let t = classify_target("src/lib.rs").unwrap();
        assert_eq!(
            t,
            WriteTarget::Workspace {
                rel_path: "src/lib.rs".into(),
            }
        );
    }

    #[test]
    fn folds_leading_dot_slash() {
        let t = classify_target("./src/lib.rs").unwrap();
        assert_eq!(
            t,
            WriteTarget::Workspace {
                rel_path: "src/lib.rs".into(),
            }
        );
    }

    #[test]
    fn classifies_job_scope_path_with_tail() {
        let t = classify_target(".codeless/jobs/foo/SCOPE.md").unwrap();
        assert_eq!(
            t,
            WriteTarget::JobScope {
                rel_path: ".codeless/jobs/foo/SCOPE.md".into(),
                segment: "foo".into(),
                tail: "SCOPE.md".into(),
            }
        );
    }

    #[test]
    fn classifies_nested_job_scope_path() {
        let t = classify_target(".codeless/jobs/foo/subdir/WORKFLOW.md").unwrap();
        assert_eq!(
            t,
            WriteTarget::JobScope {
                rel_path: ".codeless/jobs/foo/subdir/WORKFLOW.md".into(),
                segment: "foo".into(),
                tail: "subdir/WORKFLOW.md".into(),
            }
        );
    }

    #[test]
    fn job_scope_directory_target_is_none() {
        // `.codeless/jobs/foo` names a directory — a write must name
        // a file. The caller surfaces this as InvalidArgs rather than
        // routing a directory write through jobs.updateScope.
        assert!(classify_target(".codeless/jobs/foo").is_none());
    }

    #[test]
    fn unrelated_dotcodeless_path_stays_workspace() {
        // The job-scope prefix is specifically `.codeless/jobs/...`;
        // other paths under `.codeless/` (settings, future surfaces)
        // are regular workspace writes.
        let t = classify_target(".codeless/settings.json").unwrap();
        assert_eq!(
            t,
            WriteTarget::Workspace {
                rel_path: ".codeless/settings.json".into(),
            }
        );
    }

    #[test]
    fn empty_path_is_none() {
        assert!(classify_target("").is_none());
        assert!(classify_target("./").is_none());
    }
}
