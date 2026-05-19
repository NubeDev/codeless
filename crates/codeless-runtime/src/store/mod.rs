use codeless_types::Stage;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};

use crate::queue_config::QueueConfig;

mod assistant;
mod chat;
mod codec;
mod jobs;
mod personas;
mod queue;
mod repos;
mod reviews;
mod scheduled_pause_points;
mod stages;
pub mod supervisor_goals;
mod tasks;
mod todos;

pub use chat::InsertChatMessage;
pub use supervisor_goals::{
    ExecutionState, GoalAction, GoalCondition, GoalStatus, GoalValidationError, InsertGoalError,
    MarkOutcome, SupervisorGoal, SupervisorGoalId, SupervisorGoalKind, ThresholdMetric,
};
pub use todos::{TrioFailure, TrioGateOutcome};

/// SQLite-backed persistence for repos and jobs. Status enums are
/// mapped to their kebab-case wire labels (matching SCOPE.md Appendix
/// A) by explicit pattern match — the labels are wire-stable, so a
/// drift here is a wire-format break, not a refactor.
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    caps: QueueConfig,
}

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self::with_config(pool, QueueConfig::unlimited())
    }

    pub fn with_config(pool: SqlitePool, caps: QueueConfig) -> Self {
        Self { pool, caps }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn config(&self) -> QueueConfig {
        self.caps
    }
}

/// Stage row plus its rolled-up `cost_cents` (sum of child tasks) and
/// `task_count`. Returned by `list_stages_for_job` so callers don't
/// need a second query to enrich each row.
#[derive(Debug, Clone)]
pub struct StageWithCost {
    pub stage: Stage,
    pub cost_cents: i64,
    pub task_count: u32,
}

/// Open the on-disk SQLite database the production runtime uses.
///
/// Every new connection has WAL journalling, `synchronous=NORMAL`, and
/// a five-second `busy_timeout` applied via
/// `SqlitePoolOptions::after_connect`. WAL keeps readers out of writers'
/// way (the runtime fans many short reads — events, stage rollups,
/// queue polls — alongside a writer driving the state machine);
/// `NORMAL` is the documented WAL-safe sync mode and avoids the per-
/// transaction fsync stall of `FULL`; the `busy_timeout` lets short
/// contention windows resolve without surfacing `SQLITE_BUSY` to
/// callers that have no useful retry policy of their own.
///
/// In-memory tests (`sqlite::memory:`) deliberately do not call this —
/// WAL is a no-op on `:memory:` and the test harnesses already pin a
/// single connection to keep state visible across acquires.
pub async fn connect_pool(path: &std::path::Path) -> sqlx::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    SqlitePoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                conn.execute(
                    "PRAGMA journal_mode = WAL; \
                     PRAGMA synchronous = NORMAL; \
                     PRAGMA busy_timeout = 5000;",
                )
                .await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

#[cfg(test)]
mod pool_pragma_tests {
    use super::connect_pool;
    use sqlx::Row;

    #[tokio::test]
    async fn fresh_on_disk_pool_has_wal_and_busy_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pragmas.db");
        let pool = connect_pool(&path).await.expect("connect");

        let journal: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .expect("journal_mode")
            .get(0);
        assert_eq!(journal.to_ascii_lowercase(), "wal");

        let synchronous: i64 = sqlx::query("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .expect("synchronous")
            .get(0);
        // 1 == NORMAL in SQLite's PRAGMA synchronous encoding.
        assert_eq!(synchronous, 1);

        let busy_timeout: i64 = sqlx::query("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .expect("busy_timeout")
            .get(0);
        assert_eq!(busy_timeout, 5000);
    }

    #[tokio::test]
    async fn migrations_apply_on_fresh_on_disk_db() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("migrated.db");
        let pool = connect_pool(&path).await.expect("connect");
        crate::migrations::MIGRATOR
            .run(&pool)
            .await
            .expect("migrate");
    }
}
