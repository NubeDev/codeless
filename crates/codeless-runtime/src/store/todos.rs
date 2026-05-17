use codeless_types::{TaskId, Todo, TodoId, TodoKind, TodoStatus, UnixMillis};

use super::codec::{parse_todo_kind, todo_from_row, todo_kind_label, todo_status_label};
use super::SqliteStore;

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
    pub async fn update_todo_status(
        &self,
        todo_id: TodoId,
        status: TodoStatus,
        at: UnixMillis,
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
                ended_at   = COALESCE(ended_at,   ?) \
             WHERE id = ?",
        )
        .bind(todo_status_label(status))
        .bind(started_bind)
        .bind(ended_bind)
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

    /// True iff every closing-trio row (`Checks`, `Docs`, `Git`) on the
    /// task is resolved (`Done` or `Skipped`) **and** all three rows
    /// exist. The stage-completion gate calls this before emitting
    /// `StageCompleted`; the runtime injects the trio at stage entry, so
    /// "row missing" means the injection step has not yet run and the
    /// gate must keep the stage open.
    pub async fn trio_resolved(&self, task_id: TaskId) -> sqlx::Result<bool> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT kind, status FROM todos \
             WHERE task_id = ? AND kind IN ('checks', 'docs', 'git')",
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        if rows.len() != 3 {
            return Ok(false);
        }
        let mut seen = [false; 3];
        for row in rows {
            let kind: String = row.try_get("kind")?;
            let status: String = row.try_get("status")?;
            let resolved = matches!(status.as_str(), "done" | "skipped");
            if !resolved {
                return Ok(false);
            }
            let kind = parse_todo_kind(&kind)?;
            let idx = match kind {
                TodoKind::Checks => 0,
                TodoKind::Docs => 1,
                TodoKind::Git => 2,
                _ => continue,
            };
            seen[idx] = true;
        }
        Ok(seen.iter().all(|b| *b))
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
            .update_todo_status(t.id, TodoStatus::InProgress, UnixMillis(100))
            .await
            .unwrap();
        let row = store.get_todo(t.id).await.unwrap().unwrap();
        assert_eq!(row.status, TodoStatus::InProgress);
        assert_eq!(row.started_at, Some(UnixMillis(100)));
        assert_eq!(row.ended_at, None);

        store
            .update_todo_status(t.id, TodoStatus::Done, UnixMillis(200))
            .await
            .unwrap();
        let row = store.get_todo(t.id).await.unwrap().unwrap();
        assert_eq!(row.status, TodoStatus::Done);
        // First in-progress timestamp survives via COALESCE.
        assert_eq!(row.started_at, Some(UnixMillis(100)));
        assert_eq!(row.ended_at, Some(UnixMillis(200)));
    }

    #[tokio::test]
    async fn trio_resolved_requires_all_three_kinds() {
        let (store, task_id) = fresh_store_with_task().await;
        assert!(!store.trio_resolved(task_id).await.unwrap());

        let checks = todo(task_id, 10, TodoKind::Checks, "checks");
        let docs = todo(task_id, 11, TodoKind::Docs, "docs");
        let git = todo(task_id, 12, TodoKind::Git, "git");
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        // All three rows exist but still pending → not resolved.
        assert!(!store.trio_resolved(task_id).await.unwrap());

        store
            .update_todo_status(checks.id, TodoStatus::Done, UnixMillis(1))
            .await
            .unwrap();
        store
            .update_todo_status(docs.id, TodoStatus::Done, UnixMillis(2))
            .await
            .unwrap();
        // Two of three done — still not resolved.
        assert!(!store.trio_resolved(task_id).await.unwrap());

        // `Skipped` counts as resolved (the no-diff git case).
        store
            .update_todo_status(git.id, TodoStatus::Skipped, UnixMillis(3))
            .await
            .unwrap();
        assert!(store.trio_resolved(task_id).await.unwrap());
    }

    #[tokio::test]
    async fn trio_resolved_false_when_any_trio_row_failed() {
        let (store, task_id) = fresh_store_with_task().await;
        let checks = todo(task_id, 10, TodoKind::Checks, "checks");
        let docs = todo(task_id, 11, TodoKind::Docs, "docs");
        let git = todo(task_id, 12, TodoKind::Git, "git");
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        store
            .update_todo_status(checks.id, TodoStatus::Done, UnixMillis(1))
            .await
            .unwrap();
        store
            .update_todo_status(docs.id, TodoStatus::Failed, UnixMillis(2))
            .await
            .unwrap();
        store
            .update_todo_status(git.id, TodoStatus::Done, UnixMillis(3))
            .await
            .unwrap();
        assert!(!store.trio_resolved(task_id).await.unwrap());
    }
}
