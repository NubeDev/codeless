//! Applies the embedded migrator to an in-memory SQLite database and
//! checks every table/index from SCOPE.md Appendix A landed with the
//! expected column names. The point is to catch schema drift the
//! moment a migration touches the wrong column name — Phase 3 will
//! generate `sqlx::query!` calls referencing these names.

use codeless_runtime::MIGRATOR;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

async fn fresh_db() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    pool
}

async fn columns(pool: &SqlitePool, table: &str) -> Vec<String> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .expect("table_info");
    rows.into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect()
}

async fn index_names(pool: &SqlitePool) -> Vec<String> {
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("indexes");
    rows.into_iter()
        .map(|r| r.get::<String, _>("name"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrator_creates_all_tables_from_appendix_a() {
    let pool = fresh_db().await;
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();
    assert_eq!(
        tables,
        vec![
            "events".to_string(),
            "jobs".to_string(),
            "pty_sessions".to_string(),
            "repos".to_string(),
            "reviews".to_string(),
            "stages".to_string(),
            "tasks".to_string(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repos_columns_match_appendix_a() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "repos").await,
        vec![
            "id",
            "name",
            "clone_url",
            "default_branch",
            "local_path",
            "git_auth",
            "concurrency_cap",
            "default_runner",
            "created_at",
            "updated_at",
        ],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_columns_match_appendix_a() {
    let pool = fresh_db().await;
    assert_eq!(
        columns(&pool, "jobs").await,
        vec![
            "id",
            "repo_id",
            "status",
            "stop_reason",
            "template_yaml",
            "prompt",
            "runner",
            "branch",
            "worktree_path",
            "cost_cap_cents",
            "wall_clock_cap_ms",
            "cost_cents",
            "started_at",
            "ended_at",
            "created_at",
            "model",
            "permission_mode",
            "effort",
        ],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_columns_include_depends_on_and_lease_fields() {
    let pool = fresh_db().await;
    let cols = columns(&pool, "tasks").await;
    for required in [
        "depends_on",
        "lease_holder",
        "lease_expires_at",
        "cost_cents",
        "input_tokens",
        "output_tokens",
    ] {
        assert!(
            cols.contains(&required.to_string()),
            "missing {required} in tasks"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_use_cursor_autoincrement_primary_key() {
    let pool = fresh_db().await;
    sqlx::query(
        "INSERT INTO events (job_id, type, payload, created_at) \
         VALUES (NULL, 'job-started', '{}', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (job_id, type, payload, created_at) \
         VALUES (NULL, 'job-completed', '{}', 2)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let cursors: Vec<i64> = sqlx::query("SELECT cursor FROM events ORDER BY cursor")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<i64, _>("cursor"))
        .collect();
    assert_eq!(cursors, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expected_indexes_are_present() {
    let pool = fresh_db().await;
    let idx = index_names(&pool).await;
    for required in [
        "events_created_at_idx",
        "events_job_cursor_idx",
        "jobs_repo_idx",
        "jobs_status_idx",
        "pty_idle_idx",
        "reviews_status_idx",
        "stages_job_idx",
        "tasks_lease_expiry_idx",
        "tasks_stage_idx",
    ] {
        assert!(
            idx.contains(&required.to_string()),
            "missing index {required}"
        );
    }
}
