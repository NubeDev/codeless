use codeless_types::{JobId, Stage, StageStatus, UnixMillis};
use sqlx::Row;

use super::codec::{
    failure_class_label, parse_acceptance, parse_failure_class, parse_stage_status, serde_err,
    stage_status_label,
};
use super::{SqliteStore, StageWithCost};

impl SqliteStore {
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
              goal, acceptance, last_activity_at, archived, persona_id, \
              failure_class, failure_detail) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
        .bind(stage.failure_class.map(failure_class_label))
        .bind(&stage.failure_detail)
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
        failure_class: Option<codeless_types::FailureClass>,
        failure_detail: Option<&str>,
    ) -> sqlx::Result<bool> {
        // The `failure_*` columns are written on every terminal
        // transition. On `Passed` both are NULL; on `Failed` they
        // carry the class + short detail the emit site produced.
        // Writing both unconditionally (rather than guarding on
        // status) makes the SQL a single update and keeps the row
        // honest if a stage row is being replaced after a partial
        // earlier write.
        let res = sqlx::query(
            "UPDATE stages SET status = ?, ended_at = ?, \
             failure_class = ?, failure_detail = ? WHERE id = ?",
        )
        .bind(stage_status_label(status))
        .bind(ended_at.0)
        .bind(failure_class.map(failure_class_label))
        .bind(failure_detail)
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

    /// Return every stage for `job_id` along with a derived
    /// `cost_cents` (sum of the stage's `tasks.cost_cents`). Ordered
    /// by `ordinal`. The cost rollup is `0` when no tasks exist for
    /// the stage yet; the UI renders that as "—" so the user can
    /// tell "free" from "unknown".
    pub async fn list_stages_for_job(&self, job_id: JobId) -> sqlx::Result<Vec<StageWithCost>> {
        let rows = sqlx::query(
            "SELECT s.id, s.ordinal, s.name, s.status, s.verify_cmd, \
                    s.started_at, s.ended_at, s.session_id, s.goal, s.acceptance, \
                    s.last_activity_at, s.archived, s.persona_id, \
                    s.bypassed_at, s.bypassed_reason, \
                    s.failure_class, s.failure_detail, \
                    COALESCE(SUM(t.cost_cents), 0) AS cost_cents, \
                    COUNT(t.id) AS task_count \
             FROM stages s \
             LEFT JOIN tasks t ON t.stage_id = s.id \
             WHERE s.job_id = ? \
             GROUP BY s.id \
             ORDER BY s.ordinal, COALESCE(s.started_at, 0), s.id",
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
                        failure_class: row
                            .try_get::<Option<String>, _>("failure_class")?
                            .as_deref()
                            .and_then(parse_failure_class),
                        failure_detail: row.try_get("failure_detail")?,
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
        let row = sqlx::query(
            "SELECT id, job_id, ordinal, name, status, verify_cmd, \
                    started_at, ended_at, session_id, goal, acceptance, \
                    last_activity_at, archived, persona_id, \
                    bypassed_at, bypassed_reason, \
                    failure_class, failure_detail \
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
            failure_class: row
                .try_get::<Option<String>, _>("failure_class")?
                .as_deref()
                .and_then(parse_failure_class),
            failure_detail: row.try_get("failure_detail")?,
        }))
    }

    /// Flip every `running` stage row to `failed`. Crash-only call:
    /// safe exclusively at startup, before any driver loop spins up,
    /// because in steady state a `running` row corresponds to a live
    /// runner process. After a core crash the row is orphaned — no
    /// process holds it, and `latest_terminal_stage` skips it because
    /// it only considers Passed/Failed/AwaitingReview, so a fresh
    /// resume would spawn a duplicate stage at the same ordinal
    /// (observed in the wild: see the duplicate PS5 rows on job
    /// 01KRT965MV…). Flipping to `failed` makes the orphan visible
    /// to the resume path: `latest_terminal_stage` now sees it, and
    /// the operator either bypasses via `resume_job { bypass: true }`
    /// or the TemplateRunner retries it as the highest-ordinal failed
    /// row. Returns the number of rows reaped so startup can log it.
    pub async fn reap_orphan_running_stages(&self, now: UnixMillis) -> sqlx::Result<u64> {
        // The reaped row is `Failed` with `failure_class =
        // 'orphan-reap'` so the UI / CLI can distinguish a core-
        // restart interrupt from a runner-side failure and tell the
        // operator that a plain resume is safe. The detail string is
        // short and stable — the recorder has no transcript to quote
        // here.
        let res = sqlx::query(
            "UPDATE stages SET status = 'failed', ended_at = ?, \
             failure_class = 'orphan-reap', \
             failure_detail = COALESCE(failure_detail, 'core process restarted while stage was running') \
             WHERE status = 'running'",
        )
        .bind(now.0)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}
