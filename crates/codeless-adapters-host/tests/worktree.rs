//! `WorktreeManager` against a real `git init`-ed repo in a tempdir.
//! Sets `GIT_AUTHOR_*` / `GIT_COMMITTER_*` and `init.defaultBranch`
//! per-invocation so the test does not depend on the developer's
//! global git config (CI machines often have none).

use std::path::{Path, PathBuf};
use std::process::Command;

use codeless_adapters_host::worktree::{WorktreeError, WorktreeManager};
use tempfile::TempDir;

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example")
        .args(args)
        .output()
        .expect("git binary");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fresh_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_path_buf();
    git(&repo, &["init", "--initial-branch=main", "."]);
    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    git(&repo, &["add", "seed.txt"]);
    git(&repo, &["commit", "-m", "seed"]);
    (dir, repo)
}

#[test]
fn create_adds_worktree_on_a_fresh_branch() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr.create(&repo, "abc123", None).expect("create");
    assert_eq!(handle.path, base.path().join("job-abc123"));
    assert_eq!(handle.branch, "codeless/job-abc123");
    assert!(handle.path.join("seed.txt").exists());

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        listed.contains("codeless/job-abc123"),
        "worktree list did not mention the new branch: {listed}"
    );
}

#[test]
fn create_honours_requested_branch_when_non_empty() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr
        .create(&repo, "abc123", Some("feature/wizard-typed"))
        .expect("create");
    assert_eq!(handle.branch, "feature/wizard-typed");
    assert_eq!(handle.path, base.path().join("job-abc123"));
}

#[test]
fn create_falls_back_when_requested_branch_is_blank() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr.create(&repo, "abc123", Some("   ")).expect("create");
    assert_eq!(handle.branch, "codeless/job-abc123");
}

#[test]
fn create_refuses_when_path_holds_a_non_worktree_directory() {
    // A plain directory at the target path is incompatible: not a
    // git worktree, so adoption would be unsafe. `create` must
    // refuse rather than blow it away.
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let path = base.path().join("job-dup");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("foreign.txt"), "user data\n").unwrap();

    match mgr.create(&repo, "dup", None) {
        Err(WorktreeError::AlreadyExists(p)) => assert_eq!(p, path),
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

#[test]
fn create_refuses_when_existing_worktree_is_on_a_different_branch() {
    // Adoption only fires when the existing worktree's branch
    // matches what the caller asked for; otherwise the wedged job
    // would silently inherit unrelated history.
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    mgr.create(&repo, "dup", Some("feature/alpha"))
        .expect("first");
    match mgr.create(&repo, "dup", Some("feature/beta")) {
        Err(WorktreeError::AlreadyExists(p)) => assert_eq!(p, base.path().join("job-dup")),
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

#[test]
fn create_adopts_existing_worktree_on_the_same_branch() {
    // Re-running `create` for a job that's already set up — e.g. a
    // driver-loop retry after a transient failure — must succeed
    // and return the same handle rather than wedge on AlreadyExists.
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let first = mgr.create(&repo, "dup", None).expect("first");
    let adopted = mgr.create(&repo, "dup", None).expect("adopt");
    assert_eq!(first, adopted);

    // The branch stays attached: only one admin entry, not two.
    let out = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout).into_owned();
    let count = listed.matches("codeless/job-dup").count();
    assert_eq!(count, 1, "expected exactly one admin entry, got: {listed}");
}

#[test]
fn create_prunes_stale_admin_entry_then_succeeds() {
    // Crash scenario: working tree vanished from disk while git
    // still tracks it under .git/worktrees/. A naive `worktree add`
    // would fail; `create` must prune first and proceed.
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let first = mgr.create(&repo, "ghost", None).expect("first");
    std::fs::remove_dir_all(&first.path).unwrap();
    // Confirm the admin entry is still there so the test is honest
    // about what `create` has to clean up.
    let pre = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&pre.stdout).contains("codeless/job-ghost"),
        "precondition: stale admin entry should still be tracked"
    );

    let second = mgr.create(&repo, "ghost", None).expect("recreate");
    assert_eq!(second.path, first.path);
    assert!(second.path.join("seed.txt").exists());
}

#[test]
fn remove_drops_worktree_and_prunes_admin_entry() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr.create(&repo, "del", None).expect("create");
    mgr.remove(&repo, &handle.path).expect("remove");

    assert!(!handle.path.exists());
    let out = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !listed.contains("codeless/job-del"),
        "stale worktree admin entry survived prune: {listed}"
    );
}

#[test]
fn reap_orphans_removes_stale_admin_entry() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr.create(&repo, "orphan", None).expect("create");
    // Drop the working tree behind git's back to simulate a crashed
    // job whose tree was reaped externally; reap_orphans must clean
    // the admin entry so a later `create` at the same id works.
    std::fs::remove_dir_all(&handle.path).unwrap();
    mgr.reap_orphans(&repo).expect("reap");

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !listed.contains("codeless/job-orphan"),
        "reap_orphans did not prune missing tree: {listed}"
    );
}
