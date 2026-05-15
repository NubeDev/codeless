//! WORKSPACE-ATTACH milestone 2, stage 3 exit criterion:
//! the boot upsert from `--fs-root` is idempotent under the three
//! ways an operator can spell the same on-disk directory — `/a/b`,
//! `/a/b/`, and a symlink resolving to `/a/b` must all collapse onto
//! a single row in `attached_workspaces`, with one `repos` row
//! backing it.
//!
//! The collapse is enforced by the unique index on
//! `attached_workspaces.fs_root_canonical` plus the canonicalisation
//! step inside `upsert_boot_workspace`. Without the canonical column,
//! sqlite would happily store three distinct strings; the test
//! exercises that contract directly rather than trusting the index
//! to do the work alone.

use std::path::PathBuf;

use codeless_runtime::attached_workspaces::upsert_boot_workspace;
use codeless_runtime::MIGRATOR;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use tempfile::tempdir;

async fn fresh_db() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    pool
}

async fn count_rows(pool: &SqlitePool, table: &str) -> i64 {
    sqlx::query(&format!("SELECT COUNT(*) AS n FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
        .get::<i64, _>("n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boot_upsert_creates_one_row_per_canonical_path() {
    let pool = fresh_db().await;
    let dir = tempdir().unwrap();
    let real = dir.path().join("a").join("b");
    std::fs::create_dir_all(&real).unwrap();

    let first = upsert_boot_workspace(&pool, &real).await.unwrap();
    assert!(first.created_repo, "first boot should mint a repo row");
    assert!(
        first.created_attachment,
        "first boot should insert an attachment"
    );

    let second = upsert_boot_workspace(&pool, &real).await.unwrap();
    assert!(
        !second.created_repo && !second.created_attachment,
        "second identical boot must be a pure no-op: {second:?}",
    );
    assert_eq!(first.repo_id, second.repo_id);

    assert_eq!(count_rows(&pool, "attached_workspaces").await, 1);
    assert_eq!(count_rows(&pool, "repos").await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trailing_slash_and_dot_segment_collapse() {
    let pool = fresh_db().await;
    let dir = tempdir().unwrap();
    let real = dir.path().join("a").join("b");
    std::fs::create_dir_all(&real).unwrap();

    let with_slash = PathBuf::from(format!("{}/", real.display()));
    let with_dot = real.join(".");

    upsert_boot_workspace(&pool, &real).await.unwrap();
    upsert_boot_workspace(&pool, &with_slash).await.unwrap();
    upsert_boot_workspace(&pool, &with_dot).await.unwrap();

    assert_eq!(
        count_rows(&pool, "attached_workspaces").await,
        1,
        "trailing slash and `.` segment must not create new rows",
    );
    assert_eq!(count_rows(&pool, "repos").await, 1);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symlink_to_same_target_collapses() {
    let pool = fresh_db().await;
    let dir = tempdir().unwrap();
    let real = dir.path().join("a").join("b");
    std::fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("link-to-b");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let by_real = upsert_boot_workspace(&pool, &real).await.unwrap();
    let by_link = upsert_boot_workspace(&pool, &link).await.unwrap();

    assert_eq!(
        by_real.canonical, by_link.canonical,
        "symlink must canonicalise to its target",
    );
    assert_eq!(by_real.repo_id, by_link.repo_id);
    assert!(!by_link.created_repo);
    assert!(!by_link.created_attachment);
    assert_eq!(count_rows(&pool, "attached_workspaces").await, 1);
    assert_eq!(count_rows(&pool, "repos").await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boot_upsert_preserves_display_path_as_typed() {
    let pool = fresh_db().await;
    let dir = tempdir().unwrap();
    let real = dir.path().join("a").join("b");
    std::fs::create_dir_all(&real).unwrap();

    let with_slash = PathBuf::from(format!("{}/", real.display()));
    upsert_boot_workspace(&pool, &with_slash).await.unwrap();
    upsert_boot_workspace(&pool, &real).await.unwrap();

    let display: String =
        sqlx::query_scalar("SELECT fs_root_display FROM attached_workspaces LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        display.ends_with('/'),
        "first boot's display string must be preserved verbatim: {display}",
    );
}
