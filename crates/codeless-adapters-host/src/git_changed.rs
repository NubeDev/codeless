//! Enumerate the paths a worktree has touched since branching off its
//! base. The Layer-1 diff-verify pre-check (SESSION-MUTABLE-SCOPE
//! Step 2) calls this from inside a job's provisioned worktree: it
//! takes the union of *committed* changes on the worktree's branch and
//! *uncommitted* changes in the working tree, so a handover whose
//! `done` mentions a path can be checked against "did the stage
//! actually touch this file."
//!
//! Process spawning lives only in this crate per the workspace's
//! cross-platform rule; the runtime crate that wires the pre-check
//! into the REVIEW stage calls into here rather than growing a
//! `process::Command` of its own.
//!
//! Implementation choice: invoke `git` rather than a Rust git lib, for
//! the same reason `git_diff` does — share whatever the user's `git`
//! config says about line endings, rename detection, etc.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitChangedError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {op} failed (status {status}): {stderr}")]
    GitFailed {
        op: &'static str,
        status: i32,
        stderr: String,
    },
}

/// Return every path that differs in `worktree` from its branch's
/// merge-base with `base`, plus every path the working tree has
/// modified, staged, or left untracked. Paths are sorted and deduped.
///
/// `base` is the ref the worktree's branch was forked from (typically
/// the source repo's default branch — `main` / `master`). When the
/// merge-base resolution fails (no shared history, or `base` is not
/// present in this clone) the function falls back to listing every
/// commit on the current branch via `git log` against the empty tree;
/// that is the right answer for a freshly-provisioned worktree whose
/// `main` ref has been pruned but whose own commits still exist.
///
/// An empty diff returns `Ok(vec![])` — the caller distinguishes "no
/// paths changed" from "no paths to verify" with that emptiness, not
/// with an error.
pub fn changed_files(worktree: &Path, base: &str) -> Result<Vec<String>, GitChangedError> {
    let mut out: BTreeSet<String> = BTreeSet::new();

    // Committed-since-base: `git diff --name-only <base>...HEAD` uses
    // the merge-base form (three dots) so a stale `base` ref that has
    // moved on does not pollute the result with paths the worktree
    // never touched. If the ref resolution fails we fall back to
    // listing every path mentioned in every commit on the current
    // branch — better to over-report than to silently under-report.
    match run_git(
        worktree,
        "diff --name-only base...HEAD",
        &["diff", "--name-only", &format!("{base}...HEAD")],
    ) {
        Ok(s) => collect_lines(&s, &mut out),
        Err(_) => {
            let s = run_git(
                worktree,
                "log --name-only HEAD",
                &["log", "--name-only", "--pretty=format:", "HEAD"],
            )?;
            collect_lines(&s, &mut out);
        }
    }

    // Working-tree state: porcelain v1 keeps paths on the tail of each
    // line after a two-char status prefix. Renames render as
    // `R  old -> new` — keep the destination, which is what a handover
    // bullet would name.
    let porcelain = run_git(worktree, "status --porcelain", &["status", "--porcelain"])?;
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let rest = &line[3..];
        let path = match rest.split_once(" -> ") {
            Some((_, new)) => new.trim_matches('"'),
            None => rest.trim_matches('"'),
        };
        if !path.is_empty() {
            out.insert(path.to_owned());
        }
    }

    Ok(out.into_iter().collect())
}

fn collect_lines(s: &str, into: &mut BTreeSet<String>) {
    for line in s.lines() {
        let line = line.trim();
        if !line.is_empty() {
            into.insert(line.to_owned());
        }
    }
}

fn run_git(cwd: &Path, op: &'static str, args: &[&str]) -> Result<String, GitChangedError> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        return Err(GitChangedError::GitFailed {
            op,
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git(cwd: &Path, args: &[&str]) {
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

    fn seed_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "--initial-branch=main"]);
        std::fs::write(p.join("README.md"), "# seed\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "seed"]);
        dir
    }

    #[test]
    fn lists_committed_and_uncommitted_paths() {
        let dir = seed_repo();
        let p = dir.path();
        git(p, &["checkout", "-b", "feature"]);
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "add a"]);
        std::fs::write(p.join("b.txt"), "b\n").unwrap();

        let paths = changed_files(p, "main").unwrap();
        assert!(
            paths.contains(&"a.txt".to_string()),
            "missing a.txt: {paths:?}"
        );
        assert!(
            paths.contains(&"b.txt".to_string()),
            "missing b.txt: {paths:?}"
        );
    }

    #[test]
    fn empty_branch_returns_empty_set() {
        let dir = seed_repo();
        let p = dir.path();
        git(p, &["checkout", "-b", "feature"]);
        let paths = changed_files(p, "main").unwrap();
        assert!(paths.is_empty(), "expected empty diff, got {paths:?}");
    }

    #[test]
    fn renames_report_destination_path() {
        let dir = seed_repo();
        let p = dir.path();
        git(p, &["checkout", "-b", "feature"]);
        std::fs::write(p.join("old.txt"), "x\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "add old"]);
        // Rename, leave the rename uncommitted so it goes through the
        // porcelain branch (the committed branch is exercised above).
        git(p, &["mv", "old.txt", "new.txt"]);
        let paths = changed_files(p, "main").unwrap();
        assert!(
            paths.iter().any(|s| s == "new.txt"),
            "missing new.txt: {paths:?}"
        );
    }
}
