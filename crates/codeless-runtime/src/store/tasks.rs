use codeless_types::{CostCents, Task, TaskId, TaskStatus, UnixMillis};

use super::codec::{serde_err, task_from_row, task_status_label};
use super::SqliteStore;

impl SqliteStore {
    /// Best-effort upsert used by the StageRecorder when it sees a
    /// task event for a row the lease path never wrote. `INSERT OR
    /// IGNORE` so a real lease-driven row already in place wins.
    pub async fn insert_task_minimal(&self, task: &Task) -> sqlx::Result<()> {
        let depends_on = serde_json::to_string(&task.depends_on).map_err(serde_err)?;
        sqlx::query(
            "INSERT OR IGNORE INTO tasks \
             (id, stage_id, ordinal, status, depends_on, lease_holder, lease_expires_at, \
              cost_cents, input_tokens, output_tokens, started_at, ended_at) \
             VALUES (?,?,?,?,?,NULL,NULL,?,?,?,?,NULL)",
        )
        .bind(task.id.to_string())
        .bind(task.stage_id.to_string())
        .bind(task.ordinal as i64)
        .bind(task_status_label(task.status))
        .bind(&depends_on)
        .bind(task.cost_cents.0)
        .bind(task.input_tokens)
        .bind(task.output_tokens)
        .bind(task.started_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Add an `AiMessageComplete`'s cost + tokens onto a task row.
    /// Idempotency lives in the recorder, not here — every call adds.
    pub async fn add_task_cost(
        &self,
        task_id: TaskId,
        cost: CostCents,
        input_tokens: i64,
        output_tokens: i64,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE tasks SET \
                cost_cents = cost_cents + ?, \
                input_tokens = input_tokens + ?, \
                output_tokens = output_tokens + ? \
             WHERE id = ?",
        )
        .bind(cost.0)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Mark a task terminal — `status` + `ended_at`, no other fields
    /// touched. Used by the StageRecorder on `TaskCompleted`; the
    /// lease path has its own `complete_task` / `fail_task` for the
    /// "I owned this task" path.
    pub async fn mark_task_terminal(
        &self,
        task_id: TaskId,
        status: TaskStatus,
        ended_at: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE tasks SET status = ?, ended_at = ? WHERE id = ?")
            .bind(task_status_label(status))
            .bind(ended_at.0)
            .bind(task_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_task(&self, id: TaskId) -> sqlx::Result<Option<Task>> {
        let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(task_from_row).transpose()
    }
}
