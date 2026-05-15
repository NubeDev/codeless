use serde::{Deserialize, Serialize};

use crate::id::{JobId, RepoId};
use crate::time::UnixMillis;

/// An attached workspace as exposed on the wire. The canonical path
/// is the `fs.*` jail; symlinks have already been resolved server-side
/// so the UI never has to disambiguate `/var` vs `/private/var`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AttachedWorkspace {
    pub repo_id: RepoId,
    pub repo_name: String,
    /// Canonical absolute path. Symlinks resolved, no trailing slash.
    pub fs_root: String,
    pub attached_at: UnixMillis,
    /// Free-form runner kind preselected for jobs in this workspace.
    /// `None` falls back to the repo's `default_runner`, then to the
    /// global default. Free-form because runner kinds live in the
    /// adapter layer, not this wire crate.
    pub default_runner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AttachWorkspaceArgs {
    pub repo_id: RepoId,
    /// Override the repo's `local_path`. The canonicalised override
    /// must be a descendant of the canonicalised `local_path`, and
    /// dotfile directories like `.git` are rejected. When set, the
    /// override becomes the `fs.*` jail for this workspace.
    pub fs_root_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AttachWorkspaceResult {
    pub workspace: AttachedWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListWorkspacesResult {
    pub workspaces: Vec<AttachedWorkspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DetachWorkspaceArgs {
    pub repo_id: RepoId,
    pub on_running_jobs: DetachPolicy,
}

/// What detach does when jobs are still running against the workspace.
/// `Refuse` is the safe default: detach nothing, return the running
/// `JobId`s so the UI can prompt. `Stop` cancels them first.
/// `LeaveRunning` detaches only the editor surface — runners keep
/// their private worktree-scoped `fs.*` handle, but the editor side
/// loses access until re-attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DetachPolicy {
    #[default]
    Refuse,
    Stop,
    LeaveRunning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ValidateWorkspacePathArgs {
    pub path: String,
}

/// Dry-run path validation for the workspace picker. Every field is
/// independent so the UI can render the row even when the path
/// resolves but fails one of the structural checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ValidateWorkspacePathResult {
    /// `None` when the path could not be resolved at all (does not
    /// exist, traversal blocked, etc). Populated even when problems
    /// are present so the UI can show what the server saw.
    pub canonical: Option<String>,
    pub is_dir: bool,
    pub is_git_repo: bool,
    pub default_branch: Option<String>,
    pub already_attached: bool,
    pub readable: bool,
    pub writable: bool,
    pub problems: Vec<WorkspaceProblem>,
}

/// Structured reason a candidate workspace path is unusable. The UI
/// renders each variant inline rather than string-matching on a
/// generic error message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceProblem {
    NotADirectory,
    NotReadable,
    NotWritable,
    NotAGitRepo,
    InsideAnotherWorkspace {
        other_root: String,
    },
    /// `/`, `/etc`, `/usr`, `~/.ssh`, `$HOME` without a subdir, etc.
    /// Hard refusal regardless of user override.
    SystemPath,
    SymlinkOutsideHome,
}

/// Structured failure modes for attach/detach. Wire-distinct from a
/// generic `Conflict` so the UI can branch on the variant — e.g.
/// `RunningJobs` triggers the "stop jobs?" modal without parsing a
/// human-readable string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceError {
    AlreadyAttached { repo_id: RepoId, fs_root: String },
    RunningJobs { jobs: Vec<JobId> },
    PathRejected { problems: Vec<WorkspaceProblem> },
    NotAttached,
}
