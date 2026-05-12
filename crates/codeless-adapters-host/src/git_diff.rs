//! Per-file `git diff` against a base ref. The job-diff RPC drives a
//! "Files changed" tab in the UI: take a job's branch
//! (`codeless/job-<id>`), diff it against the repo's default branch
//! in the source repo on disk, and return one entry per file plus
//! its unified-diff patch.
//!
//! Implementation choice: invoke `git` rather than a Rust git lib
//! (gix, git2). The driver and `WorktreeManager` already shell out to
//! `git`; sharing the same command surface keeps behaviour consistent
//! (e.g. `core.autocrlf`, `diff.algorithm` config the user already
//! tuned for their repo apply automatically). Reading three short
//! git-output streams is fast at the file counts a single job touches.
//!
//! Process spawning lives only in this crate per the workspace's
//! cross-platform rule — the runtime calls into here rather than
//! growing a `process::Command` of its own.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitDiffError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {op} failed (status {status}): {stderr}")]
    GitFailed {
        op: &'static str,
        status: i32,
        stderr: String,
    },
    /// The base branch resolves to nothing in this repo. Surfacing
    /// this distinct from a generic `GitFailed` lets the caller map
    /// it to a friendlier message ("no `main` branch in this repo").
    #[error("base ref `{0}` not found in repo")]
    BaseMissing(String),
    /// The head branch is absent — either the job never provisioned a
    /// worktree (so no branch exists) or it was pruned manually.
    #[error("head ref `{0}` not found in repo")]
    HeadMissing(String),
}

/// One row of `git diff --numstat`. Binary files report `-` for both
/// counts in numstat output; we map that to `is_binary: true` and
/// leave the counts at zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub is_binary: bool,
    pub patch: String,
}

/// Compute the diff of `head` against `base` inside `repo_path`. The
/// returned files appear in `git`'s own ordering (which is stable per
/// invocation but not lexicographic). Empty diff returns `Ok(vec![])`.
///
/// The function calls `git` three times: once to validate refs, once
/// for `--name-status` + `--numstat` (combined as a single stream),
/// and once per file for the patch body. The per-file patch call is
/// what keeps the wire payload tractable when a diff has dozens of
/// large files — we batch on the *file* axis, not on a single huge
/// `git diff` that may exceed the SSE/REST buffer.
pub fn diff_against(
    repo_path: &Path,
    base: &str,
    head: &str,
) -> Result<Vec<DiffFile>, GitDiffError> {
    if !ref_exists(repo_path, base)? {
        return Err(GitDiffError::BaseMissing(base.to_owned()));
    }
    if !ref_exists(repo_path, head)? {
        return Err(GitDiffError::HeadMissing(head.to_owned()));
    }

    let summary = run_git(
        repo_path,
        "diff --name-status",
        &["diff", "--name-status", &format!("{base}..{head}")],
    )?;
    let numstat = run_git(
        repo_path,
        "diff --numstat",
        &["diff", "--numstat", &format!("{base}..{head}")],
    )?;

    let statuses = parse_name_status(&summary);
    let counts = parse_numstat(&numstat);

    let mut out = Vec::with_capacity(statuses.len());
    for (path, status) in statuses {
        let (additions, deletions, is_binary) = counts.get(&path).copied().unwrap_or((0, 0, false));
        let patch = if is_binary {
            String::new()
        } else {
            run_git(
                repo_path,
                "diff <file>",
                &["diff", &format!("{base}..{head}"), "--", &path],
            )?
        };
        out.push(DiffFile {
            path,
            status,
            additions,
            deletions,
            is_binary,
            patch,
        });
    }
    Ok(out)
}

fn ref_exists(repo_path: &Path, refname: &str) -> Result<bool, GitDiffError> {
    let out = Command::new("git")
        .current_dir(repo_path)
        .args(["rev-parse", "--verify", "--quiet", refname])
        .output()?;
    Ok(out.status.success())
}

fn run_git(cwd: &Path, op: &'static str, args: &[&str]) -> Result<String, GitDiffError> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        return Err(GitDiffError::GitFailed {
            op,
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse_name_status(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut it = line.splitn(2, '\t');
        let status = match it.next() {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let rest = match it.next() {
            Some(v) => v,
            None => continue,
        };
        // For renames (`R100\told\tnew`) the status carries a
        // similarity score; keep the leading letter for the wire
        // contract and use the destination path. Other statuses
        // (`A`, `M`, `D`, `T`, `C`) follow the same one-letter rule.
        let status_letter = status
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let path = if status_letter == "R" || status_letter == "C" {
            rest.split('\t').next_back().unwrap_or(rest).to_owned()
        } else {
            rest.to_owned()
        };
        out.push((path, status_letter));
    }
    out
}

fn parse_numstat(s: &str) -> std::collections::HashMap<String, (u32, u32, bool)> {
    let mut out = std::collections::HashMap::new();
    for line in s.lines() {
        let mut it = line.split('\t');
        let add = it.next().unwrap_or("");
        let del = it.next().unwrap_or("");
        let path = match it.next() {
            Some(v) => v.to_owned(),
            None => continue,
        };
        if add == "-" || del == "-" {
            out.insert(path, (0, 0, true));
        } else {
            let a = add.parse().unwrap_or(0);
            let d = del.parse().unwrap_or(0);
            out.insert(path, (a, d, false));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn run(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn fresh_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        run(p, &["init", "--initial-branch=main"]);
        std::fs::write(p.join("README.md"), "# seed\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-m", "seed"]);
        dir
    }

    #[test]
    fn empty_diff_returns_no_files() {
        let dir = fresh_repo();
        let p = dir.path();
        run(p, &["branch", "feature"]);
        let files = diff_against(p, "main", "feature").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn diff_lists_added_file_with_counts_and_patch() {
        let dir = fresh_repo();
        let p = dir.path();
        run(p, &["checkout", "-b", "feature"]);
        std::fs::write(p.join("hello.txt"), "hi\nworld\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-m", "add hello"]);
        run(p, &["checkout", "main"]);

        let files = diff_against(p, "main", "feature").unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "hello.txt");
        assert_eq!(f.status, "A");
        assert_eq!(f.additions, 2);
        assert_eq!(f.deletions, 0);
        assert!(!f.is_binary);
        assert!(f.patch.contains("+hi"));
        assert!(f.patch.contains("+world"));
    }

    #[test]
    fn missing_base_ref_is_distinct_error() {
        let dir = fresh_repo();
        let err = diff_against(dir.path(), "nope", "main").unwrap_err();
        assert!(matches!(err, GitDiffError::BaseMissing(_)));
    }

    #[test]
    fn missing_head_ref_is_distinct_error() {
        let dir = fresh_repo();
        let err = diff_against(dir.path(), "main", "nope").unwrap_err();
        assert!(matches!(err, GitDiffError::HeadMissing(_)));
    }
}
