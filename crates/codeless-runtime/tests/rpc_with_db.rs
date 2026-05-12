//! `InProcessRpc::new()` and `with_db` must both leave the pool with
//! the Appendix A schema applied — callers in stage 2+ assume they
//! can `sqlx::query!` against `repos`, `jobs`, etc. without a
//! separate migration step.

use codeless_runtime::InProcessRpc;
use sqlx::Row;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_applies_migrations_to_memory_pool() {
    let rpc = InProcessRpc::new().await.expect("init");
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(rpc.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();
    assert!(tables.contains(&"repos".into()));
    assert!(tables.contains(&"jobs".into()));
    assert!(tables.contains(&"events".into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_db_is_idempotent_for_already_migrated_pool() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let _rpc1 = InProcessRpc::with_db(pool.clone()).await.expect("first");
    let _rpc2 = InProcessRpc::with_db(pool.clone()).await.expect("second");
}
