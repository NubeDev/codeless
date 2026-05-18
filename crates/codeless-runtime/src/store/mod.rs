use codeless_types::Stage;
use sqlx::SqlitePool;

use crate::queue_config::QueueConfig;

mod assistant;
mod codec;
mod jobs;
mod personas;
mod queue;
mod repos;
mod reviews;
mod scheduled_pause_points;
mod stages;
mod tasks;
mod todos;

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
