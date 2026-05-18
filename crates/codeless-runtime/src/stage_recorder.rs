//! Subscribes to the event bus and persists `Stage` and `Task` rows
//! into SQLite so the Stages tab can query rolled-up data without
//! reconstructing it from the event stream.
//!
//! Why a separate observer instead of writing rows from the runners:
//! the runners already emit clean events ("a stage started", "a
//! message completed and cost N cents"), and the persistence
//! concern is orthogonal. Doing it here keeps the runner abstraction
//! event-shaped and gives us one place to evolve the row schema.
//!
//! Idempotency: `subscribe_since(All, None)` is live-only, so the
//! recorder picks up exactly the events fired *after* it starts. A
//! restart loses the events it would have processed during downtime;
//! the Stages tab degrades to "rows for whatever the recorder saw"
//! for the affected jobs. The events table is the durable source of
//! truth — a future backfill that replays `events` into `stages` /
//! `tasks` is a one-shot script when we need it.

use std::sync::Arc;

use codeless_rpc::RpcError;
use codeless_types::{
    CostCents, Event, Stage, StageStatus, Task, TaskId, TaskStatus, Todo, TodoId, TodoStatus,
    UnixMillis,
};
use futures_util::StreamExt;
use tokio::task::JoinHandle;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::store::SqliteStore;
use crate::time::now_ms;

/// Spawn the recorder. It tails the bus until the bus shuts down.
/// Errors during persistence are logged and skipped — losing one
/// stage row to a DB hiccup must not crash the loop.
pub async fn spawn_stage_recorder(
    bus: Arc<EventBus>,
    store: Arc<SqliteStore>,
) -> Result<JoinHandle<()>, RpcError> {
    let mut stream = bus
        .subscribe_since(SubscribeFilter::All, None)
        .await
        .map_err(|e| RpcError::Internal(format!("stage recorder subscribe: {e}")))?;
    let handle = tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let env = match item {
                Ok(env) => env,
                Err(e) => {
                    tracing::warn!(error = %e, "stage recorder stream error");
                    continue;
                }
            };
            if let Err(e) = handle_event(&store, &env).await {
                tracing::warn!(error = ?e, "stage recorder failed to persist event");
            }
        }
    });
    Ok(handle)
}

async fn handle_event(
    store: &SqliteStore,
    env: &codeless_types::EventEnvelope,
) -> sqlx::Result<()> {
    match &env.event {
        Event::StageStarted {
            stage_id,
            job_id,
            ordinal,
            name,
            persona_id,
        } => {
            store
                .insert_stage(&Stage {
                    id: *stage_id,
                    job_id: *job_id,
                    ordinal: *ordinal,
                    name: name.clone(),
                    status: StageStatus::Running,
                    verify_cmd: None,
                    started_at: Some(env.created_at),
                    ended_at: None,
                    // Filled in by the `StageSessionCaptured` handler
                    // below — the runner does not know its session id
                    // when the stage opens.
                    session_id: None,
                    // Set lazily from the template via a separate
                    // write path; the bare `StageStarted` event does
                    // not carry the authored docs fields.
                    goal: None,
                    acceptance: None,
                    // Touched by the idle sweeper / resume helper; the
                    // recorder leaves both fields at their initial
                    // values when first writing the stage row.
                    last_activity_at: Some(env.created_at),
                    archived: false,
                    // Per-stage persona override (D1). `None` means
                    // the stage inherits the job-level persona; the
                    // archive handover encodes that as `<inherited>`.
                    persona_id: persona_id.clone(),
                    // Bypass is set later via mark_stage_bypassed
                    // when resume_job's `bypass` arg is used; a fresh
                    // stage row never starts bypassed.
                    bypassed_at: None,
                    bypassed_reason: None,
                    // A fresh `StageStarted` row carries no failure
                    // info; the recorder fills these on the matching
                    // `StageCompleted` envelope via
                    // `update_stage_completed`.
                    failure_class: None,
                    failure_detail: None,
                })
                .await?;
        }
        Event::StageCompleted {
            stage_id,
            status,
            failure_class,
            failure_detail,
        } => {
            // Cap the per-stage cost view by writing `ended_at` now.
            // Status maps 1:1 between event and DB. The `bool` return
            // says whether a row was actually updated; we don't care
            // here because a missing row is logged + skipped.
            // `failure_class` / `failure_detail` are written
            // unconditionally — `None` for `Passed`, `Some(_)` for
            // `Failed` from the emit sites in `template_runner`.
            store
                .update_stage_completed(
                    *stage_id,
                    *status,
                    env.created_at,
                    *failure_class,
                    failure_detail.as_deref(),
                )
                .await?;
        }
        Event::StageSessionCaptured {
            stage_id,
            session_id,
        } if !session_id.is_empty() => {
            // Pin the runner-supplied session id onto the stage row.
            // The store's `WHERE session_id IS NULL` guard makes this a
            // first-wins write — a second envelope for the same stage
            // (replay on recorder restart, or a stray duplicate publish
            // from the runner) silently no-ops at the SQL level rather
            // than overwriting the original capture.
            store.update_stage_session_id(*stage_id, session_id).await?;
        }
        Event::TaskStarted { task_id } => {
            if let Some(stage_id) = env.stage_id {
                // Best-effort upsert: the row may already exist if
                // the lease-driven task path is also active. The
                // envelope is the source of truth for `stage_id`.
                upsert_task_started(store, *task_id, stage_id, env.created_at).await?;
            }
        }
        Event::AiMessageComplete {
            task_id,
            input_tokens,
            output_tokens,
            cost_cents,
        } => {
            // Cost accumulates: a stage often produces multiple
            // assistant messages. Tokens behave the same way.
            add_message_cost(store, *task_id, *cost_cents, *input_tokens, *output_tokens).await?;
        }
        Event::TaskCompleted { task_id, status } => {
            update_task_completed(store, *task_id, *status, env.created_at).await?;
        }
        Event::TodoAdded {
            todo_id,
            task_id,
            ordinal,
            title,
            kind,
        } => {
            // `insert_todo` is `INSERT OR IGNORE` on `(task_id, ordinal)`,
            // so a replayed `TodoAdded` on recorder restart silently
            // no-ops. The row starts in `Pending` with no started/ended
            // timestamps; the `TodoUpdated` / `TodoCompleted` arms fill
            // those columns via `update_todo_status`.
            //
            // The runtime injects the trio at stage entry, before the
            // inner adapter has had a chance to emit `TaskStarted`, so
            // the parent task row may not exist yet. Seed it with the
            // envelope's `stage_id` so the FK on `todos.task_id`
            // resolves — same pattern `add_message_cost` uses.
            if let Some(stage_id) = env.stage_id {
                if let Err(e) = upsert_task_started(store, *task_id, stage_id, env.created_at).await
                {
                    tracing::trace!(error = ?e, "todo-added task seed skipped");
                }
            }
            store
                .insert_todo(&Todo {
                    id: *todo_id,
                    task_id: *task_id,
                    ordinal: *ordinal,
                    title: title.clone(),
                    status: TodoStatus::Pending,
                    kind: *kind,
                    created_at: env.created_at,
                    started_at: None,
                    ended_at: None,
                })
                .await?;
        }
        Event::TodoUpdated { todo_id, status } => {
            update_todo(store, *todo_id, *status, env.created_at).await?;
        }
        Event::TodoCompleted { todo_id, status } => {
            update_todo(store, *todo_id, *status, env.created_at).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Insert a minimal task row if it doesn't already exist. The
/// recorder doesn't compete with the lease path — `INSERT OR IGNORE`
/// silently no-ops when `enqueue_task` got there first.
async fn upsert_task_started(
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: codeless_types::StageId,
    started_at: UnixMillis,
) -> sqlx::Result<()> {
    store
        .insert_task_minimal(&Task {
            id: task_id,
            stage_id,
            ordinal: 0,
            status: TaskStatus::Running,
            depends_on: Vec::new(),
            lease_holder: None,
            lease_expires_at: None,
            cost_cents: CostCents::ZERO,
            input_tokens: 0,
            output_tokens: 0,
            started_at: Some(started_at),
            ended_at: None,
        })
        .await?;
    Ok(())
}

async fn add_message_cost(
    store: &SqliteStore,
    task_id: TaskId,
    cost: CostCents,
    input_tokens: i64,
    output_tokens: i64,
) -> sqlx::Result<()> {
    // The task row might not exist yet if `TaskStarted` was missed
    // (recorder restarted mid-job). Seed a minimal row first so the
    // cost addition lands somewhere. We ignore errors deliberately:
    // a FK violation (orphan task event with no parent stage row)
    // means the cost-add below would also fail; we log and skip.
    if let Err(e) = store
        .insert_task_minimal(&Task {
            id: task_id,
            stage_id: codeless_types::StageId::new(),
            ordinal: 0,
            status: TaskStatus::Running,
            depends_on: Vec::new(),
            lease_holder: None,
            lease_expires_at: None,
            cost_cents: CostCents::ZERO,
            input_tokens: 0,
            output_tokens: 0,
            started_at: Some(now_ms()),
            ended_at: None,
        })
        .await
    {
        tracing::trace!(error = ?e, "stage recorder cost-add seed skipped");
    }
    store
        .add_task_cost(task_id, cost, input_tokens, output_tokens)
        .await?;
    Ok(())
}

/// Apply a `TodoUpdated` or `TodoCompleted` envelope. The store call
/// is the same for both — `update_todo_status` keys timestamp writes
/// on the supplied status, so a terminal status writes `ended_at`
/// while a non-terminal one only touches `started_at`. A miss on
/// `todo_id` (row never inserted) returns `Ok(false)`; we log and
/// move on rather than tearing the loop down.
async fn update_todo(
    store: &SqliteStore,
    todo_id: TodoId,
    status: TodoStatus,
    at: UnixMillis,
) -> sqlx::Result<()> {
    let updated = store.update_todo_status(todo_id, status, at).await?;
    if !updated {
        tracing::trace!(?todo_id, ?status, "todo status update missed unknown row");
    }
    Ok(())
}

async fn update_task_completed(
    store: &SqliteStore,
    task_id: TaskId,
    status: TaskStatus,
    ended_at: UnixMillis,
) -> sqlx::Result<()> {
    store.mark_task_terminal(task_id, status, ended_at).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use codeless_types::{Event, StageId, WorkspaceMode};

    #[tokio::test]
    async fn records_stage_start_then_complete() {
        let rpc = InProcessRpc::new().await.unwrap();
        let store = rpc.store().clone();
        let bus = rpc.bus().clone();

        // Stages have a FK to jobs, so seed a real Repo + Job first.
        let repo = codeless_types::Repo {
            id: codeless_types::RepoId::new(),
            name: "demo".into(),
            clone_url: "file:///dev/null".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: codeless_types::GitAuth::Token {
                env_var: "X".into(),
            },
            concurrency_cap: None,
            default_runner: Some("mock".into()),
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        store.insert_repo(&repo).await.unwrap();
        let job = codeless_types::Job {
            id: codeless_types::JobId::new(),
            repo_id: repo.id,
            status: codeless_types::JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let handle = spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

        let job_id = job.id;
        let stage_id = StageId::new();
        bus.publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::StageStarted {
                stage_id,
                job_id,
                ordinal: 0,
                name: "first stage".into(),
                persona_id: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
        // Give the recorder a moment to consume.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = store.list_stages_for_job(job_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stage.name, "first stage");
        assert_eq!(rows[0].stage.status, StageStatus::Running);

        bus.publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::StageCompleted {
                stage_id,
                status: StageStatus::Passed,
                failure_class: None,
                failure_detail: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = store.list_stages_for_job(job_id).await.unwrap();
        assert_eq!(rows[0].stage.status, StageStatus::Passed);
        assert!(rows[0].stage.ended_at.is_some());

        handle.abort();
    }

    #[tokio::test]
    async fn captures_stage_session_id_once() {
        let rpc = InProcessRpc::new().await.unwrap();
        let store = rpc.store().clone();
        let bus = rpc.bus().clone();

        let repo = codeless_types::Repo {
            id: codeless_types::RepoId::new(),
            name: "demo".into(),
            clone_url: "file:///dev/null".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: codeless_types::GitAuth::Token {
                env_var: "X".into(),
            },
            concurrency_cap: None,
            default_runner: Some("mock".into()),
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        store.insert_repo(&repo).await.unwrap();
        let job = codeless_types::Job {
            id: codeless_types::JobId::new(),
            repo_id: repo.id,
            status: codeless_types::JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let handle = spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

        let job_id = job.id;
        let stage_id = StageId::new();
        bus.publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::StageStarted {
                stage_id,
                job_id,
                ordinal: 0,
                name: "s".into(),
                persona_id: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
        bus.publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::StageSessionCaptured {
                stage_id,
                session_id: "sess-first".into(),
            },
            now_ms(),
        )
        .await
        .unwrap();
        // A second envelope for the same stage must be ignored: the
        // `WHERE session_id IS NULL` guard means the original capture
        // is the canonical one.
        bus.publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::StageSessionCaptured {
                stage_id,
                session_id: "sess-second".into(),
            },
            now_ms(),
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let rows = store.list_stages_for_job(job_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stage.session_id.as_deref(), Some("sess-first"));

        handle.abort();
    }

    #[tokio::test]
    async fn persists_todo_lifecycle_to_sqlite() {
        // Drives one trio item end-to-end through the bus: `TodoAdded`
        // creates the row, `TodoUpdated(InProgress)` sets `started_at`,
        // `TodoCompleted(Done)` sets `ended_at`. Confirms the recorder
        // wires the three Todo arms into the existing store methods.
        use codeless_types::{TodoKind, TodoStatus};

        let rpc = InProcessRpc::new().await.unwrap();
        let store = rpc.store().clone();
        let bus = rpc.bus().clone();

        let repo = codeless_types::Repo {
            id: codeless_types::RepoId::new(),
            name: "demo".into(),
            clone_url: "file:///dev/null".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: codeless_types::GitAuth::Token {
                env_var: "X".into(),
            },
            concurrency_cap: None,
            default_runner: Some("mock".into()),
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        store.insert_repo(&repo).await.unwrap();
        let job = codeless_types::Job {
            id: codeless_types::JobId::new(),
            repo_id: repo.id,
            status: codeless_types::JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let stage_id = StageId::new();
        let stage = Stage {
            id: stage_id,
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

        let task_id = TaskId::new();
        let task = Task {
            id: task_id,
            stage_id,
            ordinal: 0,
            status: TaskStatus::Running,
            depends_on: vec![],
            lease_holder: None,
            lease_expires_at: None,
            cost_cents: CostCents::ZERO,
            input_tokens: 0,
            output_tokens: 0,
            started_at: Some(now_ms()),
            ended_at: None,
        };
        store.insert_task_minimal(&task).await.unwrap();

        let handle = spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

        let todo_id = TodoId::new();
        bus.publish(
            Some(job.id),
            Some(stage_id),
            Some(task_id),
            Event::TodoAdded {
                todo_id,
                task_id,
                ordinal: 10,
                title: "checks".into(),
                kind: TodoKind::Checks,
            },
            UnixMillis(100),
        )
        .await
        .unwrap();
        bus.publish(
            Some(job.id),
            Some(stage_id),
            Some(task_id),
            Event::TodoUpdated {
                todo_id,
                status: TodoStatus::InProgress,
            },
            UnixMillis(200),
        )
        .await
        .unwrap();
        bus.publish(
            Some(job.id),
            Some(stage_id),
            Some(task_id),
            Event::TodoCompleted {
                todo_id,
                status: TodoStatus::Done,
            },
            UnixMillis(300),
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let row = store.get_todo(todo_id).await.unwrap().expect("todo row");
        assert_eq!(row.task_id, task_id);
        assert_eq!(row.ordinal, 10);
        assert_eq!(row.title, "checks");
        assert_eq!(row.kind, TodoKind::Checks);
        assert_eq!(row.status, TodoStatus::Done);
        assert_eq!(row.started_at, Some(UnixMillis(200)));
        assert_eq!(row.ended_at, Some(UnixMillis(300)));

        handle.abort();
    }
}
