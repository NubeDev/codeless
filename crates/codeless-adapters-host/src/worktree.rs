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
    /// the wizard.
    ///
    /// Self-healing on path collision: a prior crashed run can leave
    /// either a stale admin entry under `.git/worktrees/` (working
    /// tree gone, git still tracks it) or a real tree on disk. Both
    /// wedge a naive `git worktree add` and previously bubbled up as
    /// `AlreadyExists`, blocking driver-loop retries. So `create`
    /// now:
    /// 1. runs `git worktree prune` first to clear admin records for
    ///    trees that vanished from disk;
    /// 2. if the target path still exists and is a worktree on the
    ///    requested branch, adopts it and returns the handle —
    ///    re-running `create` for an already-set-up job is a no-op;
    /// 3. only errors with `AlreadyExists` when the path holds
    ///    something incompatible (non-worktree directory, or
    ///    worktree on a different branch). The caller has to clear
    ///    that out of band.
    pub fn create(
        &self,
        repo_path: &Path,
        job_id: &str,
        requested_branch: Option<&str>,
    ) -> Result<WorktreeHandle, WorktreeError> {
        let path = self.path_for(job_id);
        let branch = match requested_branch.map(str::trim) {
            Some(b) if !b.is_empty() => b.to_owned(),
            _ => format!("codeless/job-{job_id}"),
        };

        run_git(repo_path, "worktree prune", ["worktree", "prune"])?;

        if path.exists() {
            return match worktree_branch_at(&path) {
                Some(existing) if existing == branch => Ok(WorktreeHandle { path, branch }),
                _ => Err(WorktreeError::AlreadyExists(path)),
            };
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // After pruning a stale admin entry the branch ref typically
        // survives — `git worktree add -b` would then bail with
        // "branch already exists". Reattach to it in that case so a
        // crashed-tree-but-live-branch state is recoverable; only
        // create the branch fresh when nothing references it.
        if branch_exists(repo_path, &branch)? {
            run_git(
                repo_path,
                "worktree add",
                ["worktree", "add", path.to_str().unwrap(), &branch],
            )?;
        } else {
            run_git(
                repo_path,
                "worktree add",
                ["worktree", "add", "-b", &branch, path.to_str().unwrap()],
            )?;
        }
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

    /// Enumerate worktrees currently on disk under the manager's
    /// base directory. Filters to entries matching the `job-<id>`
    /// naming so unrelated siblings (a stray scratch dir, the
    /// `.codeless/templates/` folder when colocated) do not surface.
    /// Returns `Ok(vec![])` when the base does not exist; the GC
    /// caller treats "nothing to sweep" as a normal outcome rather
    /// than an error.
    pub fn list_on_disk(&self) -> Result<Vec<OnDiskWorktree>, WorktreeError> {
        if !self.base.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.base)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            let Some(job_id) = name_str.strip_prefix("job-") else {
                continue;
            };
            let path = entry.path();
            let meta = entry.metadata()?;
            if !meta.is_dir() {
                continue;
            }
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            let size_bytes = dir_size_bytes(&path).unwrap_or(0);
            out.push(OnDiskWorktree {
                job_id: job_id.to_owned(),
                path,
                size_bytes,
                mtime_ms,
            });
        }
        Ok(out)
    }
}

/// A worktree directory observed under the manager's base. `job_id`
/// is the directory's trailing segment after `job-`; the caller
/// joins it back to a `Job` row if it wants the source repo's path
/// (which `remove` needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDiskWorktree {
    pub job_id: String,
    pub path: PathBuf,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
}

/// Sum of file sizes under `path`, walking directories. Skips
/// entries that error mid-walk rather than failing the whole sweep
/// — partial sums are still useful and a single permission-denied
/// shouldn't blank the GC preview. Symlinks are followed at the
/// top-level entry list but not recursed into; worktrees don't
/// normally contain absolute symlinks.
fn dir_size_bytes(path: &Path) -> std::io::Result<i64> {
    let mut total: i64 = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len() as i64);
            }
        }
    }
    Ok(total)
}

/// Identify the branch of an existing worktree at `path`, or `None`
/// when `path` is not a usable worktree (plain directory, detached
/// HEAD, git not installed, etc.). Used by `create` to decide between
/// adopting a prior tree and surfacing `AlreadyExists`. Errors from
/// `git` collapse to `None` deliberately — the caller's contract is
/// "is this thing compatible?" and any non-yes answer means no.
fn worktree_branch_at(path: &Path) -> Option<String> {
    let inside = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()?;
    if !inside.status.success() {
        return None;
    }
    if String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return None;
    }
    let head = Command::new("git")
        .current_dir(path)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

/// Cheap existence check for a local branch. Used by `create` to
/// decide between `worktree add -b <branch>` (new ref) and
/// `worktree add <path> <branch>` (attach to existing ref).
fn branch_exists(repo_path: &Path, branch: &str) -> Result<bool, WorktreeError> {
    let out = Command::new("git")
        .current_dir(repo_path)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()?;
    Ok(out.status.success())
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
