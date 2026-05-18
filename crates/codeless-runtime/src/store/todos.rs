use codeless_types::{TaskId, Todo, TodoId, TodoKind, TodoStatus, UnixMillis};

use super::codec::{parse_todo_kind, todo_from_row, todo_kind_label, todo_status_label};
use super::SqliteStore;

/// Tri-state outcome of the closing-trio gate query.
///
/// `Pending` keeps the stage open; the runner polls again after a
/// short sleep. `Resolved` lets the stage emit
/// `StageCompleted { Passed }`. `Failed` carries a per-rail summary of
/// which trio rows ended `TodoStatus::Failed` (and the human-readable
/// reason if the emitter recorded one on the wire) so the runner can
/// route the stage through the auto-bypass-eligible failure path
/// instead of polling forever — the original failure mode that
/// hung job `01KRX4ZPF...` for 90 minutes on a single failed docs
/// row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrioGateOutcome {
    /// All three trio rows are `Done` or `Skipped`. Stage may pass.
    Resolved,
    /// At least one trio row is in a non-terminal status (`Pending` or
    /// `InProgress`), or the row hasn't been injected yet. Keep
    /// polling.
    Pending,
    /// At least one trio row landed `Failed`. The stage cannot pass;
    /// the caller emits `StageCompleted { Failed }` and lets the
    /// auto-bypass thrashing guard / policy decide whether to advance
    /// or halt. `failures` lists every failed rail in the order they
    /// appear in `TodoKind::TRIO` (`Checks`, `Docs`, `Git`).
    Failed { failures: Vec<TrioFailure> },
}

/// One failed trio rail. The `reason` is the latest `failure_detail`
/// the store has recorded for that row (populated when the emitter is
/// updated to write it; falls back to `None` for older rows). Used to
/// build the stage's `failure_detail` so the operator sees *which
/// rail* and *why* in the UI instead of a generic "stage failed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrioFailure {
    pub kind: TodoKind,
    pub reason: Option<String>,
}

impl SqliteStore {
    /// Idempotent insert keyed on `(task_id, ordinal)`. The StageRecorder
    /// runs as a backlog replay plus a live tail, so two `TodoAdded`
    /// envelopes for the same row are normal at startup; the
    /// `INSERT OR IGNORE` matches the pattern in `insert_task_minimal`.
    pub async fn insert_todo(&self, todo: &Todo) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO todos \
             (id, task_id, ordinal, title, status, kind, created_at, started_at, ended_at) \
             VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(todo.id.to_string())
        .bind(todo.task_id.to_string())
        .bind(todo.ordinal as i64)
        .bind(&todo.title)
        .bind(todo_status_label(todo.status))
        .bind(todo_kind_label(todo.kind))
        .bind(todo.created_at.0)
        .bind(todo.started_at.map(|t| t.0))
        .bind(todo.ended_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set a todo's status. Writes `started_at` on the first transition
    /// out of `Pending` and `ended_at` on any terminal status. Idempotent
    /// for replay — re-applying the same status is a no-op effect on the
    /// timestamp columns because `COALESCE` keeps the earlier value.
    /// `failure_detail` is persisted alongside the status flip (overwrites
    /// any prior value) so the closing-trio gate can surface "which rail
    /// and why" into the stage's `failure_detail` instead of leaving the
    /// operator with a silent stuck stage.
    pub async fn update_todo_status(
        &self,
        todo_id: TodoId,
        status: TodoStatus,
        at: UnixMillis,
        failure_detail: Option<&str>,
    ) -> sqlx::Result<bool> {
        let started_bind = (!matches!(status, TodoStatus::Pending)).then_some(at.0);
        let ended_bind = matches!(
            status,
            TodoStatus::Done | TodoStatus::Skipped | TodoStatus::Failed
        )
        .then_some(at.0);
        let res = sqlx::query(
            "UPDATE todos SET \
                status = ?, \
                started_at = COALESCE(started_at, ?), \
                ended_at   = COALESCE(ended_at,   ?), \
                failure_detail = ? \
             WHERE id = ?",
        )
        .bind(todo_status_label(status))
        .bind(started_bind)
        .bind(ended_bind)
        .bind(failure_detail)
        .bind(todo_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_todo(&self, id: TodoId) -> sqlx::Result<Option<Todo>> {
        let row = sqlx::query("SELECT * FROM todos WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(todo_from_row).transpose()
    }

    /// All todos for a task, in checklist order. Used by the
    /// `Stages` overview to render the nested rows under a tick and
    /// by the stage-completion gate to test trio resolution.
    pub async fn list_todos_for_task(&self, task_id: TaskId) -> sqlx::Result<Vec<Todo>> {
        let rows = sqlx::query("SELECT * FROM todos WHERE task_id = ? ORDER BY ordinal ASC")
            .bind(task_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(todo_from_row).collect()
    }

    /// Three-state closing-trio gate result.
    ///
    /// `Resolved` lets the stage pass; `Pending` keeps the runner
    /// polling; `Failed { failures }` carries the per-rail failure
    /// reason so the runner can route the stage through the
    /// auto-bypass-eligible failure path with a real explanation in
    /// the UI. The previous boolean form silently treated `Failed` as
    /// "not resolved" and hung the gate forever — the regression that
    /// stranded job `01KRX4ZPF...` for 90 minutes on a docs-write
    /// failure.
    pub async fn trio_gate_outcome(&self, task_id: TaskId) -> sqlx::Result<TrioGateOutcome> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT kind, status, failure_detail FROM todos \
             WHERE task_id = ? AND kind IN ('checks', 'docs', 'git')",
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        if rows.len() != 3 {
            return Ok(TrioGateOutcome::Pending);
        }
        let mut failures: Vec<TrioFailure> = Vec::new();
        let mut any_pending = false;
        let mut seen = [false; 3];
        for row in rows {
            let kind: String = row.try_get("kind")?;
            let status: String = row.try_get("status")?;
            let reason: Option<String> = row.try_get("failure_detail")?;
            let kind = parse_todo_kind(&kind)?;
            let idx = match kind {
                TodoKind::Checks => 0,
                TodoKind::Docs => 1,
                TodoKind::Git => 2,
                _ => continue,
            };
            seen[idx] = true;
            match status.as_str() {
                "done" | "skipped" => {}
                "failed" => failures.push(TrioFailure { kind, reason }),
                _ => any_pending = true,
            }
        }
        if !seen.iter().all(|b| *b) {
            return Ok(TrioGateOutcome::Pending);
        }
        // Failed wins over Pending: even if one rail is still
        // InProgress, a peer rail that already failed terminally cannot
        // un-fail, so the stage is doomed and we should surface that
        // now rather than keep polling. The previous boolean gate
        // never made this distinction.
        if !failures.is_empty() {
            return Ok(TrioGateOutcome::Failed { failures });
        }
        if any_pending {
            return Ok(TrioGateOutcome::Pending);
        }
        Ok(TrioGateOutcome::Resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::MIGRATOR;
    use codeless_types::{
        GitAuth, Job, JobStatus, Repo, RepoId, Stage, StageId, StageStatus, Task, TaskStatus,
        WorkspaceMode,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_store_with_task() -> (SqliteStore, TaskId) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool);

        let repo = Repo {
            id: RepoId::new(),
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: UnixMillis(0),
            updated_at: UnixMillis(0),
        };
        store.insert_repo(&repo).await.unwrap();

        let job = Job {
            id: codeless_types::JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: codeless_types::CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: codeless_types::CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        };
        store.insert_job(&job).await.unwrap();

        let stage = Stage {
            id: StageId::new(),
            job_id: job.id,
            ordinal: 0,
            name: "s".into(),
            status: StageStatus::Running,
            verify_cmd: None,
            started_at: None,
            ended_at: None,
            session_id: None,
            goal: None,
            acceptance: None,
            last_activity_at: None,
            archived: false,
            persona_id: None,
            failure_class: None,
            failure_detail: None,
            bypassed_at: None,
            bypassed_reason: None,
        };
        store.insert_stage(&stage).await.unwrap();

        let task = Task {
            id: TaskId::new(),
            stage_id: stage.id,
            ordinal: 0,
            status: TaskStatus::Running,
            depends_on: vec![],
            lease_holder: None,
            lease_expires_at: None,
            cost_cents: codeless_types::CostCents(0),
            input_tokens: 0,
            output_tokens: 0,
            started_at: None,
            ended_at: None,
        };
        store.insert_task_minimal(&task).await.unwrap();
        (store, task.id)
    }

    fn todo(task_id: TaskId, ordinal: u32, kind: TodoKind, title: &str) -> Todo {
        Todo {
            id: TodoId::new(),
            task_id,
            ordinal,
            title: title.into(),
            status: TodoStatus::Pending,
            kind,
            created_at: UnixMillis(0),
            started_at: None,
            ended_at: None,
            failure_detail: None,
        }
    }

    #[tokio::test]
    async fn insert_and_list_in_ordinal_order() {
        let (store, task_id) = fresh_store_with_task().await;
        store
            .insert_todo(&todo(task_id, 1, TodoKind::Runner, "second"))
            .await
            .unwrap();
        store
            .insert_todo(&todo(task_id, 0, TodoKind::Runner, "first"))
            .await
            .unwrap();
        let listed = store.list_todos_for_task(task_id).await.unwrap();
        assert_eq!(
            listed.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[tokio::test]
    async fn insert_is_idempotent_on_task_id_ordinal() {
        let (store, task_id) = fresh_store_with_task().await;
        let t = todo(task_id, 0, TodoKind::Runner, "one");
        store.insert_todo(&t).await.unwrap();
        // Different id, same (task_id, ordinal) — must not insert a duplicate.
        let mut dupe = t.clone();
        dupe.id = TodoId::new();
        dupe.title = "different".into();
        store.insert_todo(&dupe).await.unwrap();
        let listed = store.list_todos_for_task(task_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "one");
    }

    #[tokio::test]
    async fn update_status_writes_started_and_ended_timestamps() {
        let (store, task_id) = fresh_store_with_task().await;
        let t = todo(task_id, 0, TodoKind::Runner, "x");
        store.insert_todo(&t).await.unwrap();

        store
            .update_todo_status(t.id, TodoStatus::InProgress, UnixMillis(100), None)
            .await
            .unwrap();
        let row = store.get_todo(t.id).await.unwrap().unwrap();
        assert_eq!(row.status, TodoStatus::InProgress);
        assert_eq!(row.started_at, Some(UnixMillis(100)));
        assert_eq!(row.ended_at, None);

        store
            .update_todo_status(t.id, TodoStatus::Done, UnixMillis(200), None)
            .await
            .unwrap();
        let row = store.get_todo(t.id).await.unwrap().unwrap();
        assert_eq!(row.status, TodoStatus::Done);
        assert_eq!(row.started_at, Some(UnixMillis(100)));
        assert_eq!(row.ended_at, Some(UnixMillis(200)));
        assert!(row.failure_detail.is_none());
    }

    #[tokio::test]
    async fn update_status_persists_failure_detail_on_failed() {
        let (store, task_id) = fresh_store_with_task().await;
        let t = todo(task_id, 0, TodoKind::Docs, "docs");
        store.insert_todo(&t).await.unwrap();
        store
            .update_todo_status(
                t.id,
                TodoStatus::Failed,
                UnixMillis(100),
                Some("write handover: Permission denied"),
            )
            .await
            .unwrap();
        let row = store.get_todo(t.id).await.unwrap().unwrap();
        assert_eq!(row.status, TodoStatus::Failed);
        assert_eq!(
            row.failure_detail.as_deref(),
            Some("write handover: Permission denied")
        );
    }

    #[tokio::test]
    async fn trio_gate_outcome_pending_until_all_three_kinds_resolve() {
        let (store, task_id) = fresh_store_with_task().await;
        assert_eq!(
            store.trio_gate_outcome(task_id).await.unwrap(),
            TrioGateOutcome::Pending
        );

        let checks = todo(task_id, 10, TodoKind::Checks, "checks");
        let docs = todo(task_id, 11, TodoKind::Docs, "docs");
        let git = todo(task_id, 12, TodoKind::Git, "git");
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        assert_eq!(
            store.trio_gate_outcome(task_id).await.unwrap(),
            TrioGateOutcome::Pending,
            "all pending"
        );

        store
            .update_todo_status(checks.id, TodoStatus::Done, UnixMillis(1), None)
            .await
            .unwrap();
        store
            .update_todo_status(docs.id, TodoStatus::Done, UnixMillis(2), None)
            .await
            .unwrap();
        assert_eq!(
            store.trio_gate_outcome(task_id).await.unwrap(),
            TrioGateOutcome::Pending,
            "two of three"
        );

        store
            .update_todo_status(git.id, TodoStatus::Skipped, UnixMillis(3), None)
            .await
            .unwrap();
        assert_eq!(
            store.trio_gate_outcome(task_id).await.unwrap(),
            TrioGateOutcome::Resolved
        );
    }

    #[tokio::test]
    async fn trio_gate_outcome_failed_when_any_trio_row_failed() {
        // Regression test for job 01KRX4ZPF...: a failed docs trio row
        // must surface as Failed (carrying the rail's reason) so the
        // runner routes the stage through the auto-bypass-eligible
        // failure path. The previous boolean gate silently treated
        // Failed as "not resolved" and the runner polled forever.
        let (store, task_id) = fresh_store_with_task().await;
        let checks = todo(task_id, 10, TodoKind::Checks, "checks");
        let docs = todo(task_id, 11, TodoKind::Docs, "docs");
        let git = todo(task_id, 12, TodoKind::Git, "git");
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        store
            .update_todo_status(checks.id, TodoStatus::Done, UnixMillis(1), None)
            .await
            .unwrap();
        store
            .update_todo_status(
                docs.id,
                TodoStatus::Failed,
                UnixMillis(2),
                Some("write handover: disk full"),
            )
            .await
            .unwrap();
        store
            .update_todo_status(git.id, TodoStatus::Done, UnixMillis(3), None)
            .await
            .unwrap();
        let outcome = store.trio_gate_outcome(task_id).await.unwrap();
        match outcome {
            TrioGateOutcome::Failed { failures } => {
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].kind, TodoKind::Docs);
                assert_eq!(
                    failures[0].reason.as_deref(),
                    Some("write handover: disk full")
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn trio_gate_outcome_failed_wins_over_pending() {
        // Even if a peer rail is still InProgress, a terminally-failed
        // rail cannot un-fail, so the gate surfaces Failed immediately
        // rather than polling for an outcome that can no longer change.
        let (store, task_id) = fresh_store_with_task().await;
        let checks = todo(task_id, 10, TodoKind::Checks, "checks");
        let docs = todo(task_id, 11, TodoKind::Docs, "docs");
        let git = todo(task_id, 12, TodoKind::Git, "git");
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        store
            .update_todo_status(
                docs.id,
                TodoStatus::Failed,
                UnixMillis(1),
                Some("write handover: io"),
            )
            .await
            .unwrap();
        store
            .update_todo_status(checks.id, TodoStatus::InProgress, UnixMillis(2), None)
            .await
            .unwrap();
        store
            .update_todo_status(git.id, TodoStatus::Done, UnixMillis(3), None)
            .await
            .unwrap();
        match store.trio_gate_outcome(task_id).await.unwrap() {
            TrioGateOutcome::Failed { failures } => {
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].kind, TodoKind::Docs);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
