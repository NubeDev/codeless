use codeless_types::{CostCents, Task, TaskId, TaskStatus, UnixMillis};

use super::codec::{serde_err, task_from_row, task_status_label};
use super::SqliteStore;

impl SqliteStore {
    /// Enqueue a task in `enqueued` state. `lease_holder` /
    /// `lease_expires_at` / `started_at` / `ended_at` are forced to
    /// NULL regardless of what the caller put in the struct — a
    /// freshly enqueued task is by definition idle.
    pub async fn enqueue_task(&self, task: &Task) -> sqlx::Result<()> {
        let depends_on = serde_json::to_string(&task.depends_on).map_err(serde_err)?;
        sqlx::query(
            "INSERT INTO tasks \
             (id, stage_id, ordinal, status, depends_on, lease_holder, lease_expires_at, \
              cost_cents, input_tokens, output_tokens, started_at, ended_at) \
             VALUES (?,?,?,?,?,NULL,NULL,?,?,?,NULL,NULL)",
        )
        .bind(task.id.to_string())
        .bind(task.stage_id.to_string())
        .bind(task.ordinal as i64)
        .bind(task_status_label(TaskStatus::Enqueued))
        .bind(&depends_on)
        .bind(task.cost_cents.0)
        .bind(task.input_tokens)
        .bind(task.output_tokens)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomic "claim one enqueued task" for the given `runner` kind.
    /// The SELECT-and-UPDATE happens in a single statement so two
    /// callers racing on the same row cannot both win — the loser's
    /// inner SELECT returns no rows (the row's status has flipped to
    /// `running`), so its outer UPDATE matches nothing and returns
    /// `None`. Dependency satisfaction is checked inline via
    /// `json_each(tasks.depends_on)`: a task only becomes eligible
    /// once every id in its dependency array has `status='completed'`.
    /// Empty `depends_on` (linear mode) trivially satisfies that.
    pub async fn lease_next(
        &self,
        runner: &str,
        holder: &str,
        ttl_ms: i64,
        now: UnixMillis,
    ) -> sqlx::Result<Option<Task>> {
        let expires_at = now.0.saturating_add(ttl_ms);
        let global_cap = self.caps.max_global.map(|n| n as i64);
        let runner_cap = self.caps.max_per_runner.map(|n| n as i64);
        let repo_cap = self.caps.max_per_repo.map(|n| n as i64);

        // The three cap clauses each use `? IS NULL OR (count) < ?` so
        // an unlimited cap short-circuits without doing the count.
        // The counts run inside the same statement as the UPDATE,
        // which means two callers racing on the cap cannot both win
        // — SQLite serialises the writers, the second sees the first
        // claim in the running-count and is rejected by the WHERE.
        let row = sqlx::query(
            "UPDATE tasks SET \
                status = 'running', \
                lease_holder = ?, \
                lease_expires_at = ?, \
                started_at = COALESCE(started_at, ?) \
             WHERE id = ( \
                SELECT t.id FROM tasks t \
                JOIN stages s ON s.id = t.stage_id \
                JOIN jobs j ON j.id = s.job_id \
                WHERE j.runner = ? \
                  AND t.status = 'enqueued' \
                  AND (? IS NULL OR \
                       (SELECT COUNT(*) FROM tasks WHERE status='running') < ?) \
                  AND (? IS NULL OR \
                       (SELECT COUNT(*) FROM tasks tr \
                          JOIN stages sr ON sr.id = tr.stage_id \
                          JOIN jobs jr ON jr.id = sr.job_id \
                          WHERE jr.runner = j.runner \
                            AND tr.status = 'running') < ?) \
                  AND (? IS NULL OR \
                       (SELECT COUNT(*) FROM tasks tp \
                          JOIN stages sp ON sp.id = tp.stage_id \
                          JOIN jobs jp ON jp.id = sp.job_id \
                          WHERE jp.repo_id = j.repo_id \
                            AND tp.status = 'running') < ?) \
                  AND NOT EXISTS ( \
                    SELECT 1 FROM json_each(t.depends_on) je \
                    JOIN tasks dep ON dep.id = je.value \
                    WHERE dep.status != 'completed' \
                  ) \
                ORDER BY t.ordinal LIMIT 1 \
             ) \
             RETURNING *",
        )
        .bind(holder)
        .bind(expires_at)
        .bind(now.0)
        .bind(runner)
        .bind(global_cap)
        .bind(global_cap)
        .bind(runner_cap)
        .bind(runner_cap)
        .bind(repo_cap)
        .bind(repo_cap)
        .fetch_optional(&self.pool)
        .await?;
        row.map(task_from_row).transpose()
    }

    /// Mark a leased task as completed. Idempotency: a row that no
    /// longer matches `(id, lease_holder)` is silently a no-op so a
    /// retried completion after a heartbeat takeover does not flip
    /// state away from whatever the legitimate holder did.
    pub async fn complete_task(
        &self,
        task_id: TaskId,
        holder: &str,
        cost: CostCents,
        input_tokens: i64,
        output_tokens: i64,
        now: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE tasks SET \
                status = 'completed', \
                lease_holder = NULL, \
                lease_expires_at = NULL, \
                cost_cents = ?, \
                input_tokens = ?, \
                output_tokens = ?, \
                ended_at = ? \
             WHERE id = ? AND lease_holder = ?",
        )
        .bind(cost.0)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(now.0)
        .bind(task_id.to_string())
        .bind(holder)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn fail_task(
        &self,
        task_id: TaskId,
        holder: &str,
        now: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE tasks SET \
                status = 'failed', \
                lease_holder = NULL, \
                lease_expires_at = NULL, \
                ended_at = ? \
             WHERE id = ? AND lease_holder = ?",
        )
        .bind(now.0)
        .bind(task_id.to_string())
        .bind(holder)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Renew the lease on a running task held by `holder`. Returns
    /// `false` if the lease has been taken by someone else (or the
    /// task is no longer running) — the caller should abort.
    pub async fn heartbeat_task(
        &self,
        task_id: TaskId,
        holder: &str,
        new_expires_at: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query(
            "UPDATE tasks SET lease_expires_at = ? \
             WHERE id = ? AND lease_holder = ? AND status = 'running'",
        )
        .bind(new_expires_at.0)
        .bind(task_id.to_string())
        .bind(holder)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Reclaim every task whose lease expired before `now`. The row
    /// drops back to `enqueued` and the holder fields clear, making
    /// it eligible for a fresh `lease_next` call. Called at startup
    /// (per SCOPE.md "Worktrees: failed worktrees are reaped on core
    /// restart") and periodically while running.
    pub async fn release_expired_leases(&self, now: UnixMillis) -> sqlx::Result<u64> {
        let res = sqlx::query(
            "UPDATE tasks SET \
                status = 'enqueued', \
                lease_holder = NULL, \
                lease_expires_at = NULL \
             WHERE status = 'running' AND lease_expires_at < ?",
        )
        .bind(now.0)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}
