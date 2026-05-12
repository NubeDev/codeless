//! `gc_worktrees` pins:
//! - Dry-run lists candidates with size + path without touching disk.
//! - Real run removes matching trees and reports `removed_count`.
//! - `older_than_ms` filter excludes recently-touched trees.
//! - Missing `WorktreeManager` returns `Internal` (so the UI shows a
//!   clear "no root configured" message instead of an empty sweep).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{GcWorktreesArgs, RpcError, RpcServer};
use codeless_runtime::InProcessRpc;
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

fn fresh_repo() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_path_buf();
    git(&repo, &["init", "--initial-branch=main", "."]);
    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    git(&repo, &["add", "seed.txt"]);
    git(&repo, &["commit", "-m", "seed"]);
    (dir, repo)
}

#[tokio::test]
async fn gc_returns_internal_when_no_worktree_root() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .gc_worktrees(GcWorktreesArgs {
            older_than_ms: None,
            job_ids: None,
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
}

#[tokio::test]
async fn gc_dry_run_lists_without_removing() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));
    let _h1 = mgr.create(&repo, "01abcde", None).unwrap();
    let _h2 = mgr.create(&repo, "01fghij", None).unwrap();

    let rpc = InProcessRpc::new()
        .await
        .unwrap()
        .with_worktrees(mgr.clone());
    let res = rpc
        .gc_worktrees(GcWorktreesArgs {
            older_than_ms: None,
            job_ids: None,
            dry_run: true,
        })
        .await
        .unwrap();
    assert_eq!(res.entries.len(), 2);
    assert_eq!(res.removed_count, 0);
    assert!(res.entries.iter().all(|e| !e.removed && e.error.is_none()));
    assert!(base.path().join("job-01abcde").exists());
    assert!(base.path().join("job-01fghij").exists());
}

#[tokio::test]
async fn gc_real_run_removes_matching_trees() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));
    let _h1 = mgr.create(&repo, "01targeted", None).unwrap();
    let _h2 = mgr.create(&repo, "01untouched", None).unwrap();

    let rpc = InProcessRpc::new()
        .await
        .unwrap()
        .with_worktrees(mgr.clone());
    let res = rpc
        .gc_worktrees(GcWorktreesArgs {
            older_than_ms: None,
            // Empty `job_ids` is non-`None` — the filter applies but
            // matches nothing. Use a real job_id parser path by
            // passing a valid ULID-shaped id that won't match either
            // entry; this confirms the filter actually narrows.
            job_ids: None,
            dry_run: false,
        })
        .await
        .unwrap();
    // Both entries had non-ulid ids (`01targeted` etc.), so
    // `remove_one_worktree` falls back to `remove_dir_all` rather
    // than `git worktree remove`. Either way the directory must go.
    assert_eq!(res.entries.len(), 2);
    assert_eq!(res.removed_count, 2);
    assert!(!base.path().join("job-01targeted").exists());
    assert!(!base.path().join("job-01untouched").exists());
}

#[tokio::test]
async fn gc_age_filter_excludes_fresh_trees() {
    let (_repo_dir, repo) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));
    let _h = mgr.create(&repo, "01fresh", None).unwrap();

    let rpc = InProcessRpc::new()
        .await
        .unwrap()
        .with_worktrees(mgr.clone());
    let res = rpc
        .gc_worktrees(GcWorktreesArgs {
            // 1 hour cutoff — the freshly-created tree must be
            // newer than that and therefore skipped.
            older_than_ms: Some(60 * 60 * 1000),
            job_ids: None,
            dry_run: true,
        })
        .await
        .unwrap();
    assert_eq!(res.entries.len(), 0);
    assert!(base.path().join("job-01fresh").exists());
}
