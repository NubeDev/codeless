use std::str::FromStr;

use codeless_types::{
    AssistantAttachment, AssistantMessage, AssistantMessageId, AssistantMessageRole,
    AssistantThread, AssistantThreadId, CostCents, GitAuth, Job, JobId, JobStatus, Persona, Repo,
    RepoId, Review, ReviewId, ReviewStatus, Stage, StageId, StageStatus, StopReason, Task, TaskId,
    TaskStatus, UnixMillis, WorkspaceMode,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::queue_config::QueueConfig;

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

    pub async fn insert_repo(&self, repo: &Repo) -> sqlx::Result<()> {
        let git_auth = serde_json::to_string(&repo.git_auth).map_err(serde_err)?;
        sqlx::query(
            "INSERT INTO repos \
             (id, name, clone_url, default_branch, local_path, git_auth, \
              concurrency_cap, default_runner, created_at, updated_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(repo.id.to_string())
        .bind(&repo.name)
        .bind(&repo.clone_url)
        .bind(&repo.default_branch)
        .bind(&repo.local_path)
        .bind(&git_auth)
        .bind(repo.concurrency_cap)
        .bind(&repo.default_runner)
        .bind(repo.created_at.0)
        .bind(repo.updated_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_repo(&self, id: RepoId) -> sqlx::Result<Option<Repo>> {
        let row = sqlx::query("SELECT * FROM repos WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(repo_from_row).transpose()
    }

    /// Returns `true` when a row was removed, `false` when no row
    /// existed. FK violations (a repo with live jobs) surface as
    /// `sqlx::Error` — the wire contract is `ON DELETE RESTRICT`
    /// (Appendix A) and we do not auto-cascade.
    pub async fn remove_repo(&self, id: RepoId) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM repos WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_repos(&self) -> sqlx::Result<Vec<Repo>> {
        let rows = sqlx::query("SELECT * FROM repos ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(repo_from_row).collect()
    }

    pub async fn insert_job(&self, job: &Job) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO jobs \
             (id, repo_id, status, stop_reason, template_yaml, prompt, runner, branch, \
              workspace_mode, worktree_path, cost_cap_cents, wall_clock_cap_ms, cost_cents, \
              model, permission_mode, effort, system_prompt, persona_id, \
              started_at, ended_at, created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
        let res = sqlx::query(
            "UPDATE jobs SET \
                repo_id=?, status=?, stop_reason=?, template_yaml=?, prompt=?, runner=?, \
                branch=?, workspace_mode=?, worktree_path=?, cost_cap_cents=?, wall_clock_cap_ms=?, \
                cost_cents=?, model=?, permission_mode=?, effort=?, system_prompt=?, \
                persona_id=?, started_at=?, ended_at=?, created_at=? \
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
        .bind(job.started_at.map(|t| t.0))
        .bind(job.ended_at.map(|t| t.0))
        .bind(job.created_at.0)
        .bind(job.id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Hard-delete a job row and all associated events, stages, and
    /// tasks. Caller is responsible for checking the job is not
    /// running before calling.
    pub async fn delete_job(&self, id: JobId) -> sqlx::Result<bool> {
        let id_s = id.to_string();
        // Delete child rows first (events, tasks via stages, stages).
        sqlx::query("DELETE FROM events WHERE job_id = ?")
            .bind(&id_s)
            .execute(&self.pool)
            .await?;
        // Tasks reference stages, so delete tasks for this job's stages.
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
        // Reviews reference stages that reference jobs — already
        // cleaned up above via the cascade on stages.
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

    pub async fn insert_stage(&self, stage: &Stage) -> sqlx::Result<()> {
        // `INSERT OR REPLACE`: the StageRecorder runs as a backlog
        // replay + a live tail, so two StageStarted envelopes for the
        // same `StageId` are normal at startup. Idempotent upsert is
        // simpler than a conditional update + insert dance.
        // `acceptance` is JSON-encoded so the column stays a single
        // nullable TEXT. `None` writes SQL NULL — distinct from
        // `Some(vec![])`, which round-trips as `"[]"` and means "the
        // author explicitly listed no acceptance criteria yet".
        let acceptance_json = stage
            .acceptance
            .as_ref()
            .map(|a| serde_json::to_string(a).map_err(serde_err))
            .transpose()?;
        sqlx::query(
            "INSERT OR REPLACE INTO stages \
             (id, job_id, ordinal, name, status, verify_cmd, started_at, ended_at, session_id, \
              goal, acceptance, last_activity_at, archived, persona_id) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(stage.id.to_string())
        .bind(stage.job_id.to_string())
        .bind(stage.ordinal as i64)
        .bind(&stage.name)
        .bind(stage_status_label(stage.status))
        .bind(&stage.verify_cmd)
        .bind(stage.started_at.map(|t| t.0))
        .bind(stage.ended_at.map(|t| t.0))
        .bind(&stage.session_id)
        .bind(&stage.goal)
        .bind(&acceptance_json)
        .bind(stage.last_activity_at.map(|t| t.0))
        .bind(stage.archived as i64)
        .bind(&stage.persona_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// First-wins write of a runner-supplied session id onto an existing
    /// stage row. `WHERE session_id IS NULL` guards against a second
    /// runner task on the same stage clobbering the first capture, so
    /// the recorder can call this unconditionally and rely on the SQL
    /// for dedupe. Returns `true` only when the column actually
    /// transitioned NULL → `session_id`; the recorder uses that signal
    /// to know whether to emit `Event::StageSessionCaptured`.
    pub async fn update_stage_session_id(
        &self,
        id: codeless_types::StageId,
        session_id: &str,
    ) -> sqlx::Result<bool> {
        let res =
            sqlx::query("UPDATE stages SET session_id = ? WHERE id = ? AND session_id IS NULL")
                .bind(session_id)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Set `status` and `ended_at` for a stage on `StageCompleted`.
    /// `name`, `ordinal`, `started_at` are not touched — the recorder
    /// learned them at `StageStarted`. Returns `false` when the row
    /// is missing; the caller logs and continues so a late
    /// `StageCompleted` against a wiped DB does not crash the
    /// recorder.
    pub async fn update_stage_completed(
        &self,
        id: codeless_types::StageId,
        status: StageStatus,
        ended_at: codeless_types::UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE stages SET status = ?, ended_at = ? WHERE id = ?")
            .bind(stage_status_label(status))
            .bind(ended_at.0)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Mark a stage as bypassed: set `bypassed_at` and
    /// `bypassed_reason` without touching `status`. The bypass is the
    /// forward-advance signal for `TemplateRunner` on the next resume;
    /// the stage row stays `Failed` so the audit trail is honest.
    /// Returns `true` when the row exists.
    pub async fn mark_stage_bypassed(
        &self,
        id: codeless_types::StageId,
        bypassed_at: codeless_types::UnixMillis,
        reason: &str,
    ) -> sqlx::Result<bool> {
        let res =
            sqlx::query("UPDATE stages SET bypassed_at = ?, bypassed_reason = ? WHERE id = ?")
                .bind(bypassed_at.0)
                .bind(reason)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Bump `last_activity_at` on a stage. Used by the idle sweeper +
    /// the resume-resolution path to record interactive activity on the
    /// stage's warm session. Returns `true` when the row exists.
    /// Archived rows are still touched so the archive timestamp does
    /// not look stale to observers, but archive itself is one-way and
    /// touching does not un-archive.
    pub async fn touch_stage_activity(
        &self,
        id: codeless_types::StageId,
        now: codeless_types::UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE stages SET last_activity_at = ? WHERE id = ?")
            .bind(now.0)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Mark a stage's warm session as archived. Returns the prior
    /// `session_id` value when the row transitioned from
    /// `archived = 0` to `archived = 1` and had a captured session id;
    /// returns `None` when the row was already archived, missing, or
    /// had no session id to begin with. The one-shot return value is
    /// the signal `resolve_stage_resume` uses to emit
    /// `SessionArchivedThenResumed` exactly once per session boundary.
    pub async fn archive_stage_session(
        &self,
        id: codeless_types::StageId,
    ) -> sqlx::Result<Option<String>> {
        use sqlx::Row;
        let row = sqlx::query(
            "UPDATE stages SET archived = 1 \
             WHERE id = ? AND archived = 0 AND session_id IS NOT NULL \
             RETURNING session_id",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => Ok(r.try_get::<Option<String>, _>("session_id")?),
            None => Ok(None),
        }
    }

    /// Find stages whose warm session has been idle past `cutoff` and
    /// archive them in one statement. Returns the `(stage_id,
    /// prior_session_id)` pairs that transitioned so the caller can
    /// emit one `SessionArchivedThenResumed` per archived row.
    ///
    /// Idle is defined as `last_activity_at <= cutoff`: callers compute
    /// `cutoff = now - timeout` per job's `session_idle_timeout` and
    /// pass the result here. A NULL `last_activity_at` is treated as
    /// "no activity recorded" and is *not* archived — a brand-new stage
    /// that has not yet been touched should not be archived simply
    /// because it has no timestamp.
    pub async fn archive_idle_stage_sessions(
        &self,
        cutoff: codeless_types::UnixMillis,
    ) -> sqlx::Result<Vec<(codeless_types::StageId, String)>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "UPDATE stages SET archived = 1 \
             WHERE archived = 0 \
               AND session_id IS NOT NULL \
               AND last_activity_at IS NOT NULL \
               AND last_activity_at <= ? \
             RETURNING id, session_id",
        )
        .bind(cutoff.0)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id_str: String = r.try_get("id")?;
            let id: codeless_types::StageId = id_str
                .parse()
                .map_err(|e| sqlx::Error::Decode(format!("stage id: {e:?}").into()))?;
            let sid: Option<String> = r.try_get("session_id")?;
            if let Some(sid) = sid {
                out.push((id, sid));
            }
        }
        Ok(out)
    }

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

    /// Return every stage for `job_id` along with a derived
    /// `cost_cents` (sum of the stage's `tasks.cost_cents`). Ordered
    /// by `ordinal`. The cost rollup is `0` when no tasks exist for
    /// the stage yet; the UI renders that as "—" so the user can
    /// tell "free" from "unknown".
    pub async fn list_stages_for_job(&self, job_id: JobId) -> sqlx::Result<Vec<StageWithCost>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT s.id, s.ordinal, s.name, s.status, s.verify_cmd, \
                    s.started_at, s.ended_at, s.session_id, s.goal, s.acceptance, \
                    s.last_activity_at, s.archived, s.persona_id, \
                    s.bypassed_at, s.bypassed_reason, \
                    COALESCE(SUM(t.cost_cents), 0) AS cost_cents, \
                    COUNT(t.id) AS task_count \
             FROM stages s \
             LEFT JOIN tasks t ON t.stage_id = s.id \
             WHERE s.job_id = ? \
             GROUP BY s.id \
             ORDER BY s.ordinal",
        )
        .bind(job_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id_str: String = row.try_get("id")?;
                let id: codeless_types::StageId = id_str
                    .parse()
                    .map_err(|e| sqlx::Error::Decode(format!("stage id: {e:?}").into()))?;
                let status: String = row.try_get("status")?;
                Ok(StageWithCost {
                    stage: Stage {
                        id,
                        job_id,
                        ordinal: row.try_get::<i64, _>("ordinal")? as u32,
                        name: row.try_get("name")?,
                        status: parse_stage_status(&status),
                        verify_cmd: row.try_get("verify_cmd")?,
                        started_at: row
                            .try_get::<Option<i64>, _>("started_at")?
                            .map(codeless_types::UnixMillis),
                        ended_at: row
                            .try_get::<Option<i64>, _>("ended_at")?
                            .map(codeless_types::UnixMillis),
                        // Captured by the recorder on the first
                        // `Event::StageSessionCaptured` for this stage
                        // (see `update_stage_session_id`). NULL until a
                        // task on the stage reports a non-empty
                        // `RunResult.session_id`; once set, never
                        // cleared.
                        session_id: row.try_get("session_id")?,
                        goal: row.try_get("goal")?,
                        acceptance: parse_acceptance(row.try_get("acceptance")?)?,
                        last_activity_at: row
                            .try_get::<Option<i64>, _>("last_activity_at")?
                            .map(codeless_types::UnixMillis),
                        archived: row.try_get::<i64, _>("archived")? != 0,
                        persona_id: row.try_get("persona_id")?,
                        bypassed_at: row
                            .try_get::<Option<i64>, _>("bypassed_at")?
                            .map(codeless_types::UnixMillis),
                        bypassed_reason: row.try_get("bypassed_reason")?,
                    },
                    cost_cents: row.try_get::<i64, _>("cost_cents")?,
                    task_count: row.try_get::<i64, _>("task_count")? as u32,
                })
            })
            .collect()
    }

    /// Focused single-stage read, used by `TemplateRunner` to pick
    /// up the captured `session_id` for resume-aware execution
    /// (A0 — intra-stage session continuation). Returns `None` for
    /// an unknown stage id rather than erroring so the caller can
    /// fall through to a fresh-session path.
    pub async fn get_stage(&self, id: codeless_types::StageId) -> sqlx::Result<Option<Stage>> {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT id, job_id, ordinal, name, status, verify_cmd, \
                    started_at, ended_at, session_id, goal, acceptance, \
                    last_activity_at, archived, persona_id, \
                    bypassed_at, bypassed_reason \
             FROM stages WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let job_id_str: String = row.try_get("job_id")?;
        let job_id: JobId = job_id_str
            .parse()
            .map_err(|e| sqlx::Error::Decode(format!("stage.job_id: {e:?}").into()))?;
        let status: String = row.try_get("status")?;
        Ok(Some(Stage {
            id,
            job_id,
            ordinal: row.try_get::<i64, _>("ordinal")? as u32,
            name: row.try_get("name")?,
            status: parse_stage_status(&status),
            verify_cmd: row.try_get("verify_cmd")?,
            started_at: row
                .try_get::<Option<i64>, _>("started_at")?
                .map(codeless_types::UnixMillis),
            ended_at: row
                .try_get::<Option<i64>, _>("ended_at")?
                .map(codeless_types::UnixMillis),
            session_id: row.try_get("session_id")?,
            goal: row.try_get("goal")?,
            acceptance: parse_acceptance(row.try_get("acceptance")?)?,
            last_activity_at: row
                .try_get::<Option<i64>, _>("last_activity_at")?
                .map(codeless_types::UnixMillis),
            archived: row.try_get::<i64, _>("archived")? != 0,
            persona_id: row.try_get("persona_id")?,
            bypassed_at: row
                .try_get::<Option<i64>, _>("bypassed_at")?
                .map(codeless_types::UnixMillis),
            bypassed_reason: row.try_get("bypassed_reason")?,
        }))
    }

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

    pub async fn get_task(&self, id: TaskId) -> sqlx::Result<Option<Task>> {
        let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(task_from_row).transpose()
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

    pub async fn insert_review(&self, review: &Review) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO reviews (id, stage_id, status, comment, requested_at, resolved_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(review.id.to_string())
        .bind(review.stage_id.to_string())
        .bind(review_status_label(review.status))
        .bind(&review.comment)
        .bind(review.requested_at.0)
        .bind(review.resolved_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_review(&self, id: ReviewId) -> sqlx::Result<Option<Review>> {
        let row = sqlx::query("SELECT * FROM reviews WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(review_from_row).transpose()
    }

    pub async fn update_review(&self, review: &Review) -> sqlx::Result<()> {
        sqlx::query("UPDATE reviews SET status = ?, comment = ?, resolved_at = ? WHERE id = ?")
            .bind(review_status_label(review.status))
            .bind(&review.comment)
            .bind(review.resolved_at.map(|t| t.0))
            .bind(review.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List reviews, optionally narrowed by job, stage, or status. The
    /// filters compose with AND. Ordered by `requested_at` so the UI
    /// gets a stable oldest-first list. The job filter joins through
    /// `stages` so the per-job review panel does not need to map stages
    /// to jobs client-side.
    pub async fn list_reviews(
        &self,
        job_id: Option<JobId>,
        stage_id: Option<StageId>,
        status: Option<ReviewStatus>,
    ) -> sqlx::Result<Vec<Review>> {
        let status_label = status.map(review_status_label);
        let job_str = job_id.map(|j| j.to_string());
        let stage_str = stage_id.map(|s| s.to_string());
        let rows = sqlx::query(
            "SELECT reviews.* FROM reviews \
             LEFT JOIN stages ON stages.id = reviews.stage_id \
             WHERE (? IS NULL OR stages.job_id = ?) \
               AND (? IS NULL OR reviews.stage_id = ?) \
               AND (? IS NULL OR reviews.status = ?) \
             ORDER BY reviews.requested_at",
        )
        .bind(&job_str)
        .bind(&job_str)
        .bind(&stage_str)
        .bind(&stage_str)
        .bind(status_label)
        .bind(status_label)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(review_from_row).collect()
    }

    pub async fn insert_assistant_thread(&self, thread: &AssistantThread) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_threads (id, title, created_at, updated_at) \
             VALUES (?,?,?,?)",
        )
        .bind(thread.id.to_string())
        .bind(&thread.title)
        .bind(thread.created_at.0)
        .bind(thread.updated_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_assistant_thread(
        &self,
        id: AssistantThreadId,
    ) -> sqlx::Result<Option<AssistantThread>> {
        let row = sqlx::query("SELECT * FROM assistant_threads WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(assistant_thread_from_row).transpose()
    }

    /// List every assistant thread, newest-touched first. The query
    /// uses the `assistant_threads_updated_idx` (DESC) so the rail
    /// renders in stable order without a runtime sort.
    pub async fn list_assistant_threads(&self) -> sqlx::Result<Vec<AssistantThread>> {
        let rows = sqlx::query("SELECT * FROM assistant_threads ORDER BY updated_at DESC, id DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(assistant_thread_from_row).collect()
    }

    /// Delete a thread. `assistant_messages` and `assistant_attachments`
    /// cascade automatically via the FK; callers handle the on-disk
    /// attachments directory separately because the store has no
    /// filesystem handle. Returns `true` when a row was removed.
    pub async fn delete_assistant_thread(&self, id: AssistantThreadId) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM assistant_threads WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Stamp `updated_at` on a thread row without otherwise touching
    /// it. Called after a message append or attachment upload so the
    /// rail order reflects activity. No-op when the id is unknown.
    pub async fn touch_assistant_thread(
        &self,
        id: AssistantThreadId,
        when: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE assistant_threads SET updated_at = ? WHERE id = ?")
            .bind(when.0)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn insert_assistant_message(&self, message: &AssistantMessage) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, thread_id, role, content, meta_json, created_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(message.id.to_string())
        .bind(message.thread_id.to_string())
        .bind(assistant_role_label(message.role))
        .bind(&message.content)
        .bind(&message.meta_json)
        .bind(message.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch one message by id. Used by the action-card confirm/cancel
    /// path so the runtime can re-parse `meta_json` server-side rather
    /// than trust the client's claim about what's pending. Returns
    /// `None` when the row is missing (already gone, or never existed).
    pub async fn get_assistant_message(
        &self,
        id: AssistantMessageId,
    ) -> sqlx::Result<Option<AssistantMessage>> {
        let row = sqlx::query("SELECT * FROM assistant_messages WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(assistant_message_from_row).transpose()
    }

    /// Replace `meta_json` and `content` on an existing message. The
    /// action-card flow uses this to flip the status of a proposal row
    /// in place — keeping the same id and `created_at` means the rail
    /// re-renders the card with new buttons (or none) instead of
    /// growing a duplicate entry. Returns `false` if the row is gone.
    pub async fn update_assistant_message(
        &self,
        id: AssistantMessageId,
        content: &str,
        meta_json: Option<&str>,
    ) -> sqlx::Result<bool> {
        let res =
            sqlx::query("UPDATE assistant_messages SET content = ?, meta_json = ? WHERE id = ?")
                .bind(content)
                .bind(meta_json)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_assistant_messages(
        &self,
        thread_id: AssistantThreadId,
    ) -> sqlx::Result<Vec<AssistantMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_messages \
             WHERE thread_id = ? \
             ORDER BY created_at, id",
        )
        .bind(thread_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(assistant_message_from_row).collect()
    }

    pub async fn insert_assistant_attachment(&self, att: &AssistantAttachment) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_attachments \
             (id, thread_id, original_name, stored_filename, mime_type, size_bytes, created_at) \
             VALUES (?,?,?,?,?,?,?)",
        )
        .bind(att.id.to_string())
        .bind(att.thread_id.to_string())
        .bind(&att.original_name)
        .bind(&att.stored_filename)
        .bind(&att.mime_type)
        .bind(att.size_bytes)
        .bind(att.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_assistant_attachments(
        &self,
        thread_id: AssistantThreadId,
    ) -> sqlx::Result<Vec<AssistantAttachment>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_attachments \
             WHERE thread_id = ? \
             ORDER BY created_at, id",
        )
        .bind(thread_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(assistant_attachment_from_row)
            .collect()
    }

    /// Snapshot every persona row. Built-ins (`built_in = 1`) come
    /// first, ordered by id for a stable rail; user rows follow in
    /// `created_at` order so a freshly minted row lands at the bottom.
    /// JSON columns (`allowed_subagents`, `default_snippets`) are
    /// decoded here so the caller does not have to know the column
    /// shape — the wire type is `Vec<String>` either way.
    pub async fn list_personas(&self) -> sqlx::Result<Vec<Persona>> {
        let rows = sqlx::query(
            "SELECT * FROM personas \
             ORDER BY built_in DESC, \
                      CASE WHEN built_in = 1 THEN id END ASC, \
                      created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(persona_from_row).collect()
    }

    pub async fn get_persona(&self, id: &str) -> sqlx::Result<Option<Persona>> {
        let row = sqlx::query("SELECT * FROM personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(persona_from_row).transpose()
    }

    /// Upsert into the personas table. Caller supplies `now` so the
    /// runtime can hold a single timestamp across the surrounding
    /// publish; built-in rows preserve their seeded `created_at` (the
    /// `INSERT OR REPLACE` is replaced with explicit insert/update so
    /// the historical timestamp is not clobbered).
    ///
    /// `built_in` is *not* a parameter — new rows always land with
    /// `built_in = 0`, and existing rows keep whatever value they had.
    /// The runtime enforces "user cannot mint a built-in" without the
    /// schema growing a CHECK constraint.
    pub async fn upsert_persona(&self, persona: &Persona) -> sqlx::Result<Persona> {
        let allowed = serde_json::to_string(&persona.allowed_subagents).map_err(serde_err)?;
        let snippets = serde_json::to_string(&persona.default_snippets).map_err(serde_err)?;
        let existing = self.get_persona(&persona.id).await?;
        match existing {
            Some(prev) => {
                sqlx::query(
                    "UPDATE personas SET \
                        name=?, description=?, icon=?, instructions=?, \
                        use_for_jobs=?, default_model=?, allowed_subagents=?, \
                        default_snippets=?, updated_at=? \
                     WHERE id=?",
                )
                .bind(&persona.name)
                .bind(&persona.description)
                .bind(&persona.icon)
                .bind(&persona.instructions)
                .bind(persona.use_for_jobs as i64)
                .bind(&persona.default_model)
                .bind(&allowed)
                .bind(&snippets)
                .bind(persona.updated_at.0)
                .bind(&persona.id)
                .execute(&self.pool)
                .await?;
                Ok(Persona {
                    built_in: prev.built_in,
                    created_at: prev.created_at,
                    ..persona.clone()
                })
            }
            None => {
                sqlx::query(
                    "INSERT INTO personas \
                        (id, name, description, icon, instructions, use_for_jobs, \
                         default_model, allowed_subagents, default_snippets, built_in, \
                         created_at, updated_at) \
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(&persona.id)
                .bind(&persona.name)
                .bind(&persona.description)
                .bind(&persona.icon)
                .bind(&persona.instructions)
                .bind(persona.use_for_jobs as i64)
                .bind(&persona.default_model)
                .bind(&allowed)
                .bind(&snippets)
                .bind(0_i64)
                .bind(persona.created_at.0)
                .bind(persona.updated_at.0)
                .execute(&self.pool)
                .await?;
                Ok(Persona {
                    built_in: false,
                    ..persona.clone()
                })
            }
        }
    }

    /// Delete one persona row by id. Returns `true` when a row was
    /// removed. Refusing built-ins is the RPC layer's responsibility
    /// — the store happily removes whatever id it is given so tests
    /// and migrations can clean up freely.
    pub async fn delete_persona(&self, id: &str) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }
}

fn assistant_thread_from_row(row: SqliteRow) -> sqlx::Result<AssistantThread> {
    let id: String = row.try_get("id")?;
    Ok(AssistantThread {
        id: parse_id(&id)?,
        title: row.try_get("title")?,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

fn assistant_message_from_row(row: SqliteRow) -> sqlx::Result<AssistantMessage> {
    let id: String = row.try_get("id")?;
    let thread_id: String = row.try_get("thread_id")?;
    let role: String = row.try_get("role")?;
    Ok(AssistantMessage {
        id: parse_id(&id)?,
        thread_id: parse_id(&thread_id)?,
        role: parse_assistant_role(&role)?,
        content: row.try_get("content")?,
        meta_json: row.try_get("meta_json")?,
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

fn assistant_attachment_from_row(row: SqliteRow) -> sqlx::Result<AssistantAttachment> {
    let id: String = row.try_get("id")?;
    let thread_id: String = row.try_get("thread_id")?;
    Ok(AssistantAttachment {
        id: parse_id(&id)?,
        thread_id: parse_id(&thread_id)?,
        original_name: row.try_get("original_name")?,
        stored_filename: row.try_get("stored_filename")?,
        mime_type: row.try_get("mime_type")?,
        size_bytes: row.try_get("size_bytes")?,
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

fn assistant_role_label(role: AssistantMessageRole) -> &'static str {
    match role {
        AssistantMessageRole::User => "user",
        AssistantMessageRole::Assistant => "assistant",
        AssistantMessageRole::System => "system",
        AssistantMessageRole::Tool => "tool",
    }
}

fn parse_assistant_role(s: &str) -> sqlx::Result<AssistantMessageRole> {
    Ok(match s {
        "user" => AssistantMessageRole::User,
        "assistant" => AssistantMessageRole::Assistant,
        "system" => AssistantMessageRole::System,
        "tool" => AssistantMessageRole::Tool,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown assistant role: {other}").into(),
            ))
        }
    })
}

fn persona_from_row(row: SqliteRow) -> sqlx::Result<Persona> {
    let allowed_raw: String = row.try_get("allowed_subagents")?;
    let snippets_raw: String = row.try_get("default_snippets")?;
    let allowed_subagents: Vec<String> = serde_json::from_str(&allowed_raw).map_err(serde_err)?;
    let default_snippets: Vec<String> = serde_json::from_str(&snippets_raw).map_err(serde_err)?;
    let use_for_jobs: i64 = row.try_get("use_for_jobs")?;
    let built_in: i64 = row.try_get("built_in")?;
    Ok(Persona {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        icon: row.try_get("icon")?,
        instructions: row.try_get("instructions")?,
        use_for_jobs: use_for_jobs != 0,
        default_model: row.try_get("default_model")?,
        allowed_subagents,
        default_snippets,
        built_in: built_in != 0,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

fn repo_from_row(row: SqliteRow) -> sqlx::Result<Repo> {
    let id: String = row.try_get("id")?;
    let git_auth: String = row.try_get("git_auth")?;
    let git_auth: GitAuth = serde_json::from_str(&git_auth).map_err(serde_err)?;
    Ok(Repo {
        id: parse_id(&id)?,
        name: row.try_get("name")?,
        clone_url: row.try_get("clone_url")?,
        default_branch: row.try_get("default_branch")?,
        local_path: row.try_get("local_path")?,
        git_auth,
        concurrency_cap: row.try_get("concurrency_cap")?,
        default_runner: row.try_get("default_runner")?,
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
    })
}

fn job_from_row(row: SqliteRow) -> sqlx::Result<Job> {
    let id: String = row.try_get("id")?;
    let repo_id: String = row.try_get("repo_id")?;
    let status: String = row.try_get("status")?;
    let stop_reason: Option<String> = row.try_get("stop_reason")?;
    let workspace_mode: String = row.try_get("workspace_mode")?;
    let started_at: Option<i64> = row.try_get("started_at")?;
    let ended_at: Option<i64> = row.try_get("ended_at")?;
    Ok(Job {
        id: parse_id(&id)?,
        repo_id: parse_id(&repo_id)?,
        status: parse_job_status(&status)?,
        stop_reason: stop_reason.as_deref().map(parse_stop_reason).transpose()?,
        template_yaml: row.try_get("template_yaml")?,
        prompt: row.try_get("prompt")?,
        runner: row.try_get("runner")?,
        branch: row.try_get("branch")?,
        workspace_mode: parse_workspace_mode(&workspace_mode)?,
        worktree_path: row.try_get("worktree_path")?,
        cost_cap_cents: CostCents(row.try_get("cost_cap_cents")?),
        wall_clock_cap_ms: row.try_get("wall_clock_cap_ms")?,
        cost_cents: CostCents(row.try_get("cost_cents")?),
        model: row.try_get("model")?,
        permission_mode: row.try_get("permission_mode")?,
        effort: row.try_get("effort")?,
        system_prompt: row.try_get("system_prompt")?,
        persona_id: row.try_get("persona_id")?,
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

fn parse_id<T: FromStr>(s: &str) -> sqlx::Result<T>
where
    T::Err: std::fmt::Display,
{
    T::from_str(s).map_err(|e| sqlx::Error::Decode(format!("ulid decode: {e}").into()))
}

fn task_from_row(row: SqliteRow) -> sqlx::Result<Task> {
    let id: String = row.try_get("id")?;
    let stage_id: String = row.try_get("stage_id")?;
    let ordinal: i64 = row.try_get("ordinal")?;
    let status: String = row.try_get("status")?;
    let depends_on: String = row.try_get("depends_on")?;
    let depends_on: Vec<TaskId> = serde_json::from_str(&depends_on).map_err(serde_err)?;
    let lease_expires_at: Option<i64> = row.try_get("lease_expires_at")?;
    let started_at: Option<i64> = row.try_get("started_at")?;
    let ended_at: Option<i64> = row.try_get("ended_at")?;
    Ok(Task {
        id: parse_id(&id)?,
        stage_id: parse_id(&stage_id)?,
        ordinal: ordinal as u32,
        status: parse_task_status(&status)?,
        depends_on,
        lease_holder: row.try_get("lease_holder")?,
        lease_expires_at: lease_expires_at.map(UnixMillis),
        cost_cents: CostCents(row.try_get("cost_cents")?),
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        started_at: started_at.map(UnixMillis),
        ended_at: ended_at.map(UnixMillis),
    })
}

fn stage_status_label(s: StageStatus) -> &'static str {
    match s {
        StageStatus::Pending => "pending",
        StageStatus::Running => "running",
        StageStatus::AwaitingReview => "awaiting-review",
        StageStatus::Passed => "passed",
        StageStatus::Failed => "failed",
    }
}

/// Decode the JSON-encoded `stages.acceptance` column. `None` (SQL
/// NULL) and `Some` (a JSON array literal) are kept distinct so the
/// wire round-trip preserves "field omitted" vs. "field set to empty
/// list" — the UI overview reads the empty-list case as "stage has no
/// acceptance criteria yet", which is different from "this stage
/// predates the field".
fn parse_acceptance(raw: Option<String>) -> sqlx::Result<Option<Vec<String>>> {
    raw.map(|s| serde_json::from_str::<Vec<String>>(&s).map_err(serde_err))
        .transpose()
}

fn parse_stage_status(s: &str) -> StageStatus {
    match s {
        "running" => StageStatus::Running,
        "awaiting-review" => StageStatus::AwaitingReview,
        "passed" => StageStatus::Passed,
        "failed" => StageStatus::Failed,
        _ => StageStatus::Pending,
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

fn task_status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Enqueued => "enqueued",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn parse_task_status(s: &str) -> sqlx::Result<TaskStatus> {
    Ok(match s {
        "enqueued" => TaskStatus::Enqueued,
        "running" => TaskStatus::Running,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown task status: {other}").into(),
            ))
        }
    })
}

fn job_status_label(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Draft => "draft",
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::AwaitingReview => "awaiting-review",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Stopped => "stopped",
        JobStatus::Paused => "paused",
    }
}

fn parse_job_status(s: &str) -> sqlx::Result<JobStatus> {
    Ok(match s {
        "draft" => JobStatus::Draft,
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "awaiting-review" => JobStatus::AwaitingReview,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "stopped" => JobStatus::Stopped,
        "paused" => JobStatus::Paused,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown job status: {other}").into(),
            ))
        }
    })
}

fn stop_reason_label(s: StopReason) -> &'static str {
    match s {
        StopReason::User => "user",
        StopReason::CostCap => "cost-cap",
        StopReason::WallClock => "wall-clock",
        StopReason::RunnerCrash => "runner-crash",
    }
}

fn workspace_mode_label(m: WorkspaceMode) -> &'static str {
    match m {
        WorkspaceMode::InRepo => "in-repo",
        WorkspaceMode::Worktree => "worktree",
    }
}

fn parse_workspace_mode(s: &str) -> sqlx::Result<WorkspaceMode> {
    Ok(match s {
        "in-repo" => WorkspaceMode::InRepo,
        "worktree" => WorkspaceMode::Worktree,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown workspace_mode: {other}").into(),
            ))
        }
    })
}

fn review_status_label(s: ReviewStatus) -> &'static str {
    match s {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Approved => "approved",
        ReviewStatus::Rejected => "rejected",
        ReviewStatus::Stopped => "stopped",
        ReviewStatus::RerunRequested => "rerun-requested",
    }
}

fn parse_review_status(s: &str) -> sqlx::Result<ReviewStatus> {
    Ok(match s {
        "pending" => ReviewStatus::Pending,
        "approved" => ReviewStatus::Approved,
        "rejected" => ReviewStatus::Rejected,
        "stopped" => ReviewStatus::Stopped,
        "rerun-requested" => ReviewStatus::RerunRequested,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown review status: {other}").into(),
            ))
        }
    })
}

fn review_from_row(row: SqliteRow) -> sqlx::Result<Review> {
    let id: String = row.try_get("id")?;
    let stage_id: String = row.try_get("stage_id")?;
    let status: String = row.try_get("status")?;
    let resolved_at: Option<i64> = row.try_get("resolved_at")?;
    Ok(Review {
        id: parse_id(&id)?,
        stage_id: parse_id(&stage_id)?,
        status: parse_review_status(&status)?,
        comment: row.try_get("comment")?,
        requested_at: UnixMillis(row.try_get("requested_at")?),
        resolved_at: resolved_at.map(UnixMillis),
    })
}

fn parse_stop_reason(s: &str) -> sqlx::Result<StopReason> {
    Ok(match s {
        "user" => StopReason::User,
        "cost-cap" => StopReason::CostCap,
        "wall-clock" => StopReason::WallClock,
        "runner-crash" => StopReason::RunnerCrash,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown stop reason: {other}").into(),
            ))
        }
    })
}

fn serde_err(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(format!("json: {e}").into())
}
