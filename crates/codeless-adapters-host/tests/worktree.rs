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

    let handle = mgr.create(&repo, "abc123").expect("create");
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
fn create_refuses_if_path_already_exists() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    mgr.create(&repo, "dup").expect("first");
    match mgr.create(&repo, "dup") {
        Err(WorktreeError::AlreadyExists(p)) => assert_eq!(p, base.path().join("job-dup")),
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

#[test]
fn remove_drops_worktree_and_prunes_admin_entry() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = WorktreeManager::new(base.path());

    let handle = mgr.create(&repo, "del").expect("create");
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

    let handle = mgr.create(&repo, "orphan").expect("create");
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
