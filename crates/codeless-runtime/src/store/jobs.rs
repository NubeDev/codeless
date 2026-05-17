use codeless_types::{Job, JobId, RepoId};

use super::codec::{
    encode_auto_bypass_policy, job_from_row, job_status_label, stop_reason_label,
    workspace_mode_label,
};
use super::SqliteStore;

impl SqliteStore {
    pub async fn insert_job(&self, job: &Job) -> sqlx::Result<()> {
        let auto_bypass_policy = encode_auto_bypass_policy(job.auto_bypass_policy.as_ref())?;
        sqlx::query(
            "INSERT INTO jobs \
             (id, repo_id, status, stop_reason, template_yaml, prompt, runner, branch, \
              workspace_mode, worktree_path, cost_cap_cents, wall_clock_cap_ms, cost_cents, \
              model, permission_mode, effort, system_prompt, persona_id, \
              auto_bypass_policy, pending_operator_comment, precheck_override_once, started_at, ended_at, created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(job.id.to_string())
        .bind(job.repo_id.to_string())
        .bind(job_status_label(job.status))
        .bind(job.stop_reason.map(stop_reason_label))
        .bind(&job.template_yaml)
        .bind(&job.prompt)
        .bind(&job.runner)
        .bind(&job.branch)
        .bind(workspace_mode_label(job.workspace_mode))
        .bind(&job.worktree_path)
        .bind(job.cost_cap_cents.0)
        .bind(job.wall_clock_cap_ms)
        .bind(job.cost_cents.0)
        .bind(&job.model)
        .bind(&job.permission_mode)
        .bind(&job.effort)
        .bind(&job.system_prompt)
        .bind(&job.persona_id)
        .bind(&auto_bypass_policy)
        .bind(&job.pending_operator_comment)
        .bind(i64::from(job.precheck_override_once))
        .bind(job.started_at.map(|t| t.0))
        .bind(job.ended_at.map(|t| t.0))
        .bind(job.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_job(&self, id: JobId) -> sqlx::Result<Option<Job>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(job_from_row).transpose()
    }

    /// Whole-row update by primary key. Returns `true` when a row
    /// was updated.
    pub async fn update_job(&self, job: &Job) -> sqlx::Result<bool> {
        let auto_bypass_policy = encode_auto_bypass_policy(job.auto_bypass_policy.as_ref())?;
        let res = sqlx::query(
            "UPDATE jobs SET \
                repo_id=?, status=?, stop_reason=?, template_yaml=?, prompt=?, runner=?, \
                branch=?, workspace_mode=?, worktree_path=?, cost_cap_cents=?, wall_clock_cap_ms=?, \
                cost_cents=?, model=?, permission_mode=?, effort=?, system_prompt=?, \
                persona_id=?, auto_bypass_policy=?, pending_operator_comment=?, precheck_override_once=?, started_at=?, ended_at=?, created_at=? \
             WHERE id=?",
        )
        .bind(job.repo_id.to_string())
        .bind(job_status_label(job.status))
        .bind(job.stop_reason.map(stop_reason_label))
        .bind(&job.template_yaml)
        .bind(&job.prompt)
        .bind(&job.runner)
        .bind(&job.branch)
        .bind(workspace_mode_label(job.workspace_mode))
        .bind(&job.worktree_path)
        .bind(job.cost_cap_cents.0)
        .bind(job.wall_clock_cap_ms)
        .bind(job.cost_cents.0)
        .bind(&job.model)
        .bind(&job.permission_mode)
        .bind(&job.effort)
        .bind(&job.system_prompt)
        .bind(&job.persona_id)
        .bind(&auto_bypass_policy)
        .bind(&job.pending_operator_comment)
        .bind(i64::from(job.precheck_override_once))
        .bind(job.started_at.map(|t| t.0))
        .bind(job.ended_at.map(|t| t.0))
        .bind(job.created_at.0)
        .bind(job.id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Stash an operator comment on the job row so the next runner
    /// build picks it up. Overwrites any prior unconsumed comment
    /// because a fresh resume call expresses fresh operator intent;
    /// the prior text would otherwise leak into a stage the operator
    /// did not write it for. `None` clears the slot explicitly.
    pub async fn set_pending_operator_comment(
        &self,
        job_id: JobId,
        comment: Option<&str>,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE jobs SET pending_operator_comment = ? WHERE id = ?")
            .bind(comment)
            .bind(job_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Atomically read and clear the pending operator comment. The
    /// runner factory calls this once per `build()` so a single
    /// resume comment threads into exactly one runner instance — a
    /// subsequent rebuild (e.g. driver restart, second resume
    /// without a fresh comment) sees `None` rather than re-applying
    /// stale guidance to the wrong stage. SQLite's `RETURNING`
    /// makes the read+clear a single statement, no transaction
    /// gymnastics needed.
    pub async fn take_pending_operator_comment(
        &self,
        job_id: JobId,
    ) -> sqlx::Result<Option<String>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "UPDATE jobs \
             SET pending_operator_comment = NULL \
             WHERE id = ? AND pending_operator_comment IS NOT NULL \
             RETURNING pending_operator_comment",
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(c,)| c))
    }

    /// One-shot opt-in to advance past a REVIEW stage's diff-verify
    /// pre-check failure. Persisted on the job row so a driver
    /// restart between the operator's click and the next runner
    /// build does not lose the authorisation; consumed atomically by
    /// `take_precheck_override_once` on the runner side so the flag
    /// burns down after exactly one re-attempt.
    pub async fn set_precheck_override_once(&self, job_id: JobId) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE jobs SET precheck_override_once = 1 WHERE id = ?")
            .bind(job_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Atomically read-and-clear the override flag. Returns `true`
    /// exactly once after the operator opted in; subsequent calls
    /// return `false` until the operator opts in again. The single
    /// `UPDATE ... RETURNING` statement keeps the read+clear race-
    /// free without a transaction, matching the
    /// `take_pending_operator_comment` pattern.
    pub async fn take_precheck_override_once(&self, job_id: JobId) -> sqlx::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "UPDATE jobs \
             SET precheck_override_once = 0 \
             WHERE id = ? AND precheck_override_once = 1 \
             RETURNING precheck_override_once",
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Hard-delete a job row and all associated events, stages, and
    /// tasks. Caller is responsible for checking the job is not
    /// running before calling.
    pub async fn delete_job(&self, id: JobId) -> sqlx::Result<bool> {
        let id_s = id.to_string();
        sqlx::query("DELETE FROM events WHERE job_id = ?")
            .bind(&id_s)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM tasks WHERE stage_id IN \
             (SELECT id FROM stages WHERE job_id = ?)",
        )
        .bind(&id_s)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM stages WHERE job_id = ?")
            .bind(&id_s)
            .execute(&self.pool)
            .await?;
        // Reviews FK on `stages`; the row above cascaded the join key,
        // so any review whose stage no longer exists is orphaned and
        // gets swept here.
        sqlx::query(
            "DELETE FROM reviews WHERE stage_id NOT IN \
             (SELECT id FROM stages)",
        )
        .execute(&self.pool)
        .await?;
        let res = sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(&id_s)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_jobs(&self, repo: Option<RepoId>) -> sqlx::Result<Vec<Job>> {
        let rows = match repo {
            Some(r) => {
                sqlx::query("SELECT * FROM jobs WHERE repo_id = ? ORDER BY created_at")
                    .bind(r.to_string())
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query("SELECT * FROM jobs ORDER BY created_at")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.into_iter().map(job_from_row).collect()
    }

    /// Returns `Some(job)` if there is already an `in_repo` job for
    /// `repo_id` in a non-terminal state. Used by `submit_job` to
    /// enforce the one-in_repo-per-repo invariant.
    pub async fn active_in_repo_job(&self, repo_id: RepoId) -> sqlx::Result<Option<Job>> {
        let row = sqlx::query(
            "SELECT * FROM jobs \
             WHERE repo_id = ? \
               AND workspace_mode = 'in-repo' \
               AND status NOT IN ('completed', 'failed', 'stopped') \
             LIMIT 1",
        )
        .bind(repo_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(job_from_row).transpose()
    }

    /// Return every `running` job row to `queued`. Crash-only,
    /// startup-only twin of `reap_orphan_running_stages`. The
    /// `replay_backlog` pass only picks up `Queued` rows, so a job
    /// stuck `Running` after a core crash would otherwise stay
    /// invisible to the driver forever. `stop_reason` is left alone:
    /// it is already `None` for a `Running` row by construction (set
    /// by `pause_job` / `stop_job`, both of which transition out of
    /// `Running` first), so nothing to clear.
    pub async fn reap_orphan_running_jobs(&self) -> sqlx::Result<u64> {
        let res = sqlx::query("UPDATE jobs SET status = 'queued' WHERE status = 'running'")
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}
