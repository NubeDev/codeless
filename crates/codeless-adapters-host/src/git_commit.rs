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

/// Resolve `HEAD` to a full commit SHA. Used by callers that need to
/// link back to a commit they just produced with `commit_paths` — for
/// the UI patch-approval flow, the SHA travels on the
/// `ScopePatchApproved` / `ScopePatchRejected` event so the inbox can
/// render a `commit/<sha>` link without a follow-up shell-out.
pub fn head_sha(repo: &Path) -> Result<String, GitCommitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "rev-parse",
            source: e,
        })?;
    if !out.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "rev-parse",
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Outcome of looking up a previously-resolved scope patch in `git
/// log`. The UI's idempotent-call path uses this to render
/// `ScopePatchActionResult::AlreadyResolved` without rerunning the
/// approve/reject side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorPatchResolution {
    Approved { commit_sha: String },
    Rejected { commit_sha: String },
}

/// Search the repo's commit history for the most recent commit that
/// resolved a scope patch with the given id. Matches the commit-body
/// markers the runtime / CLI emit:
///
/// - `Approved scope patch <id>.` → `PriorPatchResolution::Approved`
/// - `Rejected scope patch <id>.` → `PriorPatchResolution::Rejected`
///
/// Returns `Ok(None)` when no such commit exists — the caller treats
/// that as "this id was never queued, surface it as `NotFound`".
pub fn find_patch_resolution(
    repo: &Path,
    patch_id: &str,
) -> Result<Option<PriorPatchResolution>, GitCommitError> {
    let pattern = format!("scope patch {patch_id}");
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--all",
            "-F",
            "--grep",
            pattern.as_str(),
            "--format=%H%n%B%n<<<END-COMMIT>>>",
            "-n",
            "32",
        ])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "log --grep",
            source: e,
        })?;
    if !out.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "log --grep",
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let approved_marker = format!("Approved scope patch {patch_id}");
    let rejected_marker = format!("Rejected scope patch {patch_id}");
    for record in text.split("<<<END-COMMIT>>>") {
        let record = record.trim_start_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let (sha, body) = match record.split_once('\n') {
            Some((sha, rest)) => (sha.trim(), rest),
            None => continue,
        };
        if body.contains(&approved_marker) {
            return Ok(Some(PriorPatchResolution::Approved {
                commit_sha: sha.to_owned(),
            }));
        }
        if body.contains(&rejected_marker) {
            return Ok(Some(PriorPatchResolution::Rejected {
                commit_sha: sha.to_owned(),
            }));
        }
    }
    Ok(None)
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
        .arg("-f")
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

/// Stage every change in the worktree (`git add -A`, respecting
/// `.gitignore`) and commit with `subject`. Returns `Ok(true)` if a
/// commit was produced, `Ok(false)` if `git add -A` produced no
/// staged diff — the no-op case the per-stage commit step maps onto
/// the `Git` trio rail's `Skipped` status.
///
/// Why this exists separately from `commit_paths`: the runner-driven
/// per-stage commit doesn't track which files the agent touched, so
/// it wants "stage everything that changed". `commit_paths` uses
/// `git add -f -- <paths>` which force-stages *past* `.gitignore` —
/// that's correct for the `.codeless/jobs/<name>.yaml` callers (the
/// job dir may be ignored) but catastrophic for a "." path against
/// a developer worktree with a multi-gigabyte `target/`. This
/// function never passes `-f`, so build artefacts in `.gitignore`
/// stay out of the commit.
pub fn commit_all_changes(repo: &Path, subject: &str) -> Result<bool, GitCommitError> {
    let add = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["add", "-A"])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "add -A",
            source: e,
        })?;
    if !add.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "add -A",
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

/// Push the current branch to `origin`, setting the upstream the first
/// time. Used by the per-stage commit step so the operator can see
/// stage progress on the remote without polling the worktree by hand
/// — without this, the `Git` trio rail's "commit + push" contract was
/// commit-only and stages were invisible on GitHub until manual
/// intervention.
///
/// Implementation: `git push -u origin HEAD`. `HEAD` rather than a
/// named branch keeps the call site oblivious to the worktree's branch
/// name (which is already encoded in the worktree checkout). `-u` is
/// idempotent: it sets `branch.<name>.remote` and `.merge` the first
/// time, and is a no-op once the upstream is configured.
///
/// Auth: the worktree shares the source repo's remote and credential
/// configuration, so whatever `git push` does in the developer's
/// terminal is what happens here. There is no token-injection step;
/// credential helpers, SSH keys, and `GIT_ASKPASS` all work as the
/// user already has them set up.
pub fn push_current_branch(repo: &Path) -> Result<(), GitCommitError> {
    let push = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["push", "-u", "origin", "HEAD"])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "push",
            source: e,
        })?;
    if !push.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "push",
            status: push.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&push.stderr).into_owned(),
        });
    }
    Ok(())
}

/// Revert a previously-produced commit by SHA, producing a new
/// `Revert "<original subject>"` commit on the current branch. Used by
/// the UI patch-inbox's 10-second undo toast: when the operator clicks
/// Approve and then changes their mind, the toast's `[Undo]` button
/// runs this against the approval SHA so both events are preserved in
/// `git log` (decision OQ#3 — "audit trail records both the approval
/// and the undo").
///
/// The commit is created non-interactively (`--no-edit`); the message
/// is whatever `git revert` produces by default. The runtime returns
/// the new commit's SHA so the UI can display "approval undone" with a
/// link to the revert commit.
pub fn git_revert(repo: &Path, sha: &str) -> Result<String, GitCommitError> {
    let revert = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["revert", "--no-edit", sha])
        .output()
        .map_err(|e| GitCommitError::Io {
            op: "revert",
            source: e,
        })?;
    if !revert.status.success() {
        return Err(GitCommitError::GitFailed {
            op: "revert",
            status: revert.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&revert.stderr).into_owned(),
        });
    }
    head_sha(repo)
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
    fn git_revert_undoes_an_earlier_commit_and_returns_new_sha() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        fs::write(&p, "hi").unwrap();
        commit_paths(tmp.path(), "add hello.md", std::slice::from_ref(&p)).unwrap();
        let approval_sha = head_sha(tmp.path()).unwrap();

        let revert_sha = git_revert(tmp.path(), &approval_sha).unwrap();
        assert_ne!(revert_sha, approval_sha, "revert must produce a new commit");

        // The file the original commit added is gone after the revert.
        assert!(!p.exists(), "revert should remove hello.md");

        // Both the original commit and its revert remain in `git log`,
        // which is the audit-trail invariant decision OQ#3 records.
        let log = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["log", "--pretty=%s"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout);
        assert!(log.lines().any(|l| l == "add hello.md"), "log: {log}");
        assert!(
            log.lines()
                .any(|l| l.starts_with("Revert \"add hello.md\"")),
            "log: {log}"
        );
    }

    #[test]
    fn push_current_branch_lands_a_commit_on_origin() {
        // A local bare repo plays the role of `origin`. Real
        // deployments push to GitHub; this test only proves the shell-
        // out semantics — that the commit reaches the configured
        // upstream and that the first push sets `branch.*.remote`.
        let origin = TempDir::new().unwrap();
        let out = Command::new("git")
            .arg("-C")
            .arg(origin.path())
            .args(["init", "-q", "--bare", "-b", "main"])
            .output()
            .unwrap();
        assert!(out.status.success(), "init --bare: {out:?}");

        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let add_remote = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["remote", "add", "origin"])
            .arg(origin.path())
            .output()
            .unwrap();
        assert!(add_remote.status.success(), "remote add: {add_remote:?}");

        let p = tmp.path().join("hello.md");
        fs::write(&p, "hi").unwrap();
        commit_paths(tmp.path(), "add hello.md", std::slice::from_ref(&p)).unwrap();

        push_current_branch(tmp.path()).unwrap();

        let log = Command::new("git")
            .arg("-C")
            .arg(origin.path())
            .args(["log", "--pretty=%s", "main"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout);
        assert!(
            log.lines().any(|l| l == "add hello.md"),
            "origin log missing commit: {log}"
        );

        // Second push with no new commits is a no-op success — proves
        // the `-u` upstream-set is idempotent and that an "everything
        // up-to-date" exit is not flagged as failure.
        push_current_branch(tmp.path()).unwrap();
    }

    #[test]
    fn push_current_branch_returns_error_when_no_remote() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        fs::write(&p, "hi").unwrap();
        commit_paths(tmp.path(), "add hello.md", std::slice::from_ref(&p)).unwrap();
        let err = push_current_branch(tmp.path()).unwrap_err();
        assert!(
            matches!(err, GitCommitError::GitFailed { op: "push", .. }),
            "expected GitFailed{{op:\"push\"}}, got {err:?}"
        );
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
