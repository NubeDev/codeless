use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Manages per-job `git worktree` checkouts. SCOPE.md "Workspace =
/// one `git worktree` per job" drives the design:
/// - `create` adds a worktree at `<base>/job-<id>` on a fresh branch
///   `codeless/job-<id>`.
/// - `remove` runs `git worktree remove --force` followed by `prune`
///   so a half-deleted tree on disk does not leave behind a stale
///   admin entry in `.git/worktrees/`.
/// - `reap_orphans` runs `git worktree prune` on the source repo at
///   startup; SCOPE.md "Worktrees: failed worktrees are reaped on
///   core restart". Disk reclamation of the working tree itself is
///   deliberately user-driven — the default is preservation so the
///   user can inspect a crashed job's state.
///
/// All operations shell out to `git`. This is the **only** crate in the
/// workspace permitted to spawn processes; mobile builds depend only
/// on `codeless-types` and `codeless-client` and therefore cannot
/// reach this module.
pub struct WorktreeManager {
    base: PathBuf,
}

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {op} failed (status {status}): {stderr}")]
    GitFailed {
        op: &'static str,
        status: i32,
        stderr: String,
    },
    #[error("worktree already exists at {0}")]
    AlreadyExists(PathBuf),
}

/// Result of `create`. Carries both the on-disk path and the branch
/// name so callers do not have to re-derive them from the job id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeHandle {
    pub path: PathBuf,
    pub branch: String,
}

impl WorktreeManager {
    /// `base` is the directory under which per-job worktrees are
    /// created — typically `~/.local/share/codeless/worktrees/`
    /// (SCOPE.md "Per-user file layout"). Tests point at a tempdir.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn path_for(&self, job_id: &str) -> PathBuf {
        self.base.join(format!("job-{job_id}"))
    }

    /// Create a worktree from `repo_path` on a fresh branch. The
    /// caller passes the desired branch name via `requested_branch`;
    /// when `None` or empty after trimming, the manager falls back
    /// to `codeless/job-<job_id>`. The fallback covers callers that
    /// don't track per-job branch preferences (e.g. test harnesses)
    /// while letting `submit_job` honour the user-typed branch from
    /// the wizard. Errors with `AlreadyExists` rather than
    /// overwriting — the caller should pick a new job id or remove
    /// the stale tree first.
    pub fn create(
        &self,
        repo_path: &Path,
        job_id: &str,
        requested_branch: Option<&str>,
    ) -> Result<WorktreeHandle, WorktreeError> {
        let path = self.path_for(job_id);
        if path.exists() {
            return Err(WorktreeError::AlreadyExists(path));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let branch = match requested_branch.map(str::trim) {
            Some(b) if !b.is_empty() => b.to_owned(),
            _ => format!("codeless/job-{job_id}"),
        };
        run_git(
            repo_path,
            "worktree add",
            ["worktree", "add", "-b", &branch, path.to_str().unwrap()],
        )?;
        Ok(WorktreeHandle { path, branch })
    }

    /// Remove the worktree at `path` and prune the source repo's
    /// admin entries. `--force` because the working tree may contain
    /// uncommitted job output; this is the documented MVP behaviour
    /// (SCOPE.md "Worktrees: removal is destructive — user must
    /// preserve before reaping").
    pub fn remove(&self, repo_path: &Path, path: &Path) -> Result<(), WorktreeError> {
        if path.exists() {
            run_git(
                repo_path,
                "worktree remove",
                ["worktree", "remove", "--force", path.to_str().unwrap()],
            )?;
        }
        run_git(repo_path, "worktree prune", ["worktree", "prune"])?;
        Ok(())
    }

    /// Drop admin entries for worktrees whose directories no longer
    /// exist. Called once on core startup; cheap and idempotent.
    pub fn reap_orphans(&self, repo_path: &Path) -> Result<(), WorktreeError> {
        run_git(repo_path, "worktree prune", ["worktree", "prune"])?;
        Ok(())
    }
}

fn run_git<I, S>(cwd: &Path, op: &'static str, args: I) -> Result<(), WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        return Err(WorktreeError::GitFailed {
            op,
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(())
}
