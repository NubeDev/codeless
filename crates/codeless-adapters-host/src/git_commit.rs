//! Commit specific paths to a repo with a fixed subject line. The
//! job-file surface in `codeless-runtime` calls this after every
//! write/delete/migrate so the user's repo carries a real audit trail
//! (`git log -- .codeless/jobs/<name>/`).
//!
//! Why a dedicated helper instead of inlining `Command::new("git")`
//! at the call site: process spawn is restricted to this crate by R1
//! in `DOCS/SCOPE.md` / `CLAUDE.md`. The runtime call sites pass paths
//! and a subject; the host adapter owns the actual git invocation.
//!
//! Behaviour: `git add` the given paths, then `git commit -m <subject>`.
//! A failing `git add` or `git commit` returns `GitCommitError`. The
//! "nothing to commit" case is *not* an error — the call is a no-op
//! when the index is clean after staging. This keeps the call site
//! idempotent: writing the same content twice does not panic.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// What can go wrong staging or committing the requested paths. The
/// runtime maps these into `RpcError::Internal` with the stderr text
/// included so an operator can debug from the wire response.
#[derive(Debug, Error)]
pub enum GitCommitError {
    #[error("git io ({op}): {source}")]
    Io {
        op: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("git {op} failed ({status}): {stderr}")]
    GitFailed {
        op: &'static str,
        status: i32,
        stderr: String,
    },
}

/// Stage the given paths and create a commit with `subject`. Returns
/// `Ok(true)` if a commit was produced, `Ok(false)` if there was
/// nothing to commit after staging. Both outcomes are success — the
/// caller is asserting "make the working tree reflect what's on disk
/// for these paths", and a no-op commit means the working tree
/// already matched.
///
/// `paths` are joined to `repo` (passed as `-C <repo>` to git); they
/// may be absolute or relative. The caller is responsible for keeping
/// them inside the repo — there's no `..` check here because every
/// call site has already sanitised input through `job_dir`.
pub fn commit_paths(repo: &Path, subject: &str, paths: &[PathBuf]) -> Result<bool, GitCommitError> {
    if paths.is_empty() {
        return Ok(false);
    }

    let add = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("add")
        .arg("--")
        .args(paths)
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "add",
            source: e,
        })?;
    if !add.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "add",
            status: add.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&add.stderr).into_owned(),
        });
    }

    let staged = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["diff", "--cached", "--quiet"])
        .status()
        .map_err(|e| GitCommitError::Io {
            op: "diff --cached",
            source: e,
        })?;
    if staged.success() {
        return Ok(false);
    }

    let commit = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["commit", "-m", subject])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "commit",
            source: e,
        })?;
    if !commit.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "commit",
            status: commit.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&commit.stderr).into_owned(),
        });
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_repo(dir: &Path) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "test"][..],
            &["commit", "--allow-empty", "-q", "-m", "root"][..],
        ] {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args.iter().copied())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {:?}", out);
        }
    }

    #[test]
    fn commit_paths_creates_commit_for_new_file() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        fs::write(&p, "hi").unwrap();
        let made = commit_paths(tmp.path(), "add hello.md", &[p]).unwrap();
        assert!(made);
        let log = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["log", "--pretty=%s"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout);
        assert!(log.lines().any(|l| l == "add hello.md"), "log: {log}");
    }

    #[test]
    fn commit_paths_is_noop_when_nothing_changed() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        fs::write(&p, "hi").unwrap();
        commit_paths(tmp.path(), "add hello.md", std::slice::from_ref(&p)).unwrap();
        let made = commit_paths(tmp.path(), "no change", &[p]).unwrap();
        assert!(!made);
    }
}
