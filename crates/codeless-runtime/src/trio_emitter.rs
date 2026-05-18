//! Helpers that flip the runtime-injected trio rows (`Checks`, `Docs`,
//! `Git`) on a stage's terminal task. The runtime owns those rows —
//! see `template_runner::publish_trio` — so the three runtime emit
//! sites (`verify_runner` for `Checks`, the handover writer for `Docs`,
//! and the per-stage commit step for `Git`) call into this module to
//! flip status without each site re-deriving "which `TodoId` does the
//! `Docs` row on this task have".
//!
//! Resolution policy: look up the trio `TodoId` by `(task_id, kind)`
//! through `SqliteStore::list_todos_for_task` at emit time. The
//! `publish_trio` site allocates the IDs and drops them; re-querying
//! here keeps each emit site standalone — the alternative (threading
//! the three IDs out of `publish_trio` and through every caller) ties
//! a lot of code together for an effect a single index hit already
//! satisfies.
//!
//! Missing row policy: a `find_trio_id` miss is a soft skip with a
//! trace-level log, not an error. The trio is injected at stage entry
//! by the runtime; the only way the row is missing is the test path
//! where `publish_trio` never ran, in which case emitting a wire event
//! against an absent row would be a lie that the recorder would then
//! log as an "unknown todo" update.

use codeless_types::{Event, StageId, TaskId, TodoId, TodoKind, TodoStatus};

use crate::runner::RunnerContext;
use crate::store::SqliteStore;
use crate::time::now_ms;

/// Look up the trio row of `kind` for `task_id`. Returns `None` when
/// the row is absent (trio not yet injected) or when the store query
/// errors — both cases collapse to "no emit" at the call site.
pub async fn find_trio_id(store: &SqliteStore, task_id: TaskId, kind: TodoKind) -> Option<TodoId> {
    match store.list_todos_for_task(task_id).await {
        Ok(todos) => todos.into_iter().find(|t| t.kind == kind).map(|t| t.id),
        Err(err) => {
            tracing::warn!(?err, %task_id, ?kind, "trio emitter: list_todos_for_task failed");
            None
        }
    }
}

/// Publish `TodoUpdated { InProgress }` for the trio row of `kind` on
/// `task_id`. Silently no-ops when the row is missing; the bus publish
/// is best-effort.
pub async fn emit_trio_started(
    ctx: &RunnerContext,
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: StageId,
    kind: TodoKind,
) {
    // Scoped pause hook — `Before` trio todo. A `pause_points:` entry
    // like `{ stage: 3, todo: docs, position: before }` halts the job
    // exactly here, before the per-kind runtime emit-site (handover
    // writer for `Docs`, `verify_runner` for `Checks`, the commit
    // step for `Git`) does its work. The hook returns `Paused` after
    // it has already flipped the job row and fired the cancel token,
    // so the caller (the per-kind site) sees `ctx.cancel` next time
    // it checks and bails.
    let outcome = crate::scoped_pause_hook::check_trio(
        store,
        ctx.bus.as_ref(),
        ctx.job_id,
        stage_id,
        kind,
        codeless_types::pause_point::PausePointPosition::Before,
        &ctx.cancel,
    )
    .await;
    if outcome == crate::scoped_pause_hook::HookOutcome::Paused {
        tracing::info!(
            ?kind,
            %stage_id,
            "trio started: scoped pause point fired; skipping in-progress emit"
        );
        return;
    }

    let Some(todo_id) = find_trio_id(store, task_id, kind).await else {
        tracing::trace!(?kind, %task_id, "trio started: row not present; skipping emit");
        return;
    };
    publish(
        ctx,
        stage_id,
        task_id,
        Event::TodoUpdated {
            todo_id,
            status: TodoStatus::InProgress,
        },
    )
    .await;
}

/// Publish `TodoCompleted { status }` for the trio row of `kind` on
/// `task_id`. The caller maps its terminal outcome onto the
/// `TodoStatus` (`Done` for success, `Skipped` for the no-diff git
/// case, `Failed` otherwise) — the closing-trio gate treats `Done`
/// and `Skipped` as resolved, and routes `Failed` into the stage's
/// auto-bypass-eligible failure path with `failure_detail` surfaced
/// to the operator.
pub async fn emit_trio_completed(
    ctx: &RunnerContext,
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: StageId,
    kind: TodoKind,
    status: TodoStatus,
    failure_detail: Option<String>,
) {
    let Some(todo_id) = find_trio_id(store, task_id, kind).await else {
        tracing::trace!(?kind, %task_id, "trio completed: row not present; skipping emit");
        return;
    };
    publish(
        ctx,
        stage_id,
        task_id,
        Event::TodoCompleted {
            todo_id,
            status,
            failure_detail,
        },
    )
    .await;

    // Scoped pause hook — `After` trio todo. Fires after the
    // TodoCompleted lands so the row is on the wire (and in SQLite)
    // before the pause divider does. The hook is best-effort: if the
    // store call fails we log and continue, so a transient DB hiccup
    // doesn't strand the trio between Done and the next stage.
    let outcome = crate::scoped_pause_hook::check_trio(
        store,
        ctx.bus.as_ref(),
        ctx.job_id,
        stage_id,
        kind,
        codeless_types::pause_point::PausePointPosition::After,
        &ctx.cancel,
    )
    .await;
    if outcome == crate::scoped_pause_hook::HookOutcome::Paused {
        tracing::info!(
            ?kind,
            %stage_id,
            "trio completed: scoped pause point fired"
        );
    }
}

/// Per-stage commit seam. Wraps
/// `codeless_adapters_host::commit_all_changes` (which does
/// `git add -A` + `git commit`, respecting `.gitignore`) and emits
/// the `Git` trio updates around it: `InProgress` before the commit,
/// then `Done` (commit produced), `Skipped` (no diff after staging —
/// the workflow's no-op case documented in `todos.rs`), or `Failed`
/// (git surfaced an error). Returns the underlying commit outcome
/// so the caller can keep going on `Ok(false)` and bail on `Err`.
/// The shell-out is synchronous (process spawn lives in
/// `codeless-adapters-host` per R1); the offload through
/// `spawn_blocking` keeps the reactor unblocked on a slow worktree.
///
/// Why `commit_all_changes` and not `commit_paths`: the runner does
/// not track which files the agent touched this stage, so it asks
/// git for the answer. The older `commit_paths` helper uses
/// `git add -f -- <paths>` which force-stages past `.gitignore` —
/// fine for its existing `.codeless/jobs/<name>.yaml` callers (the
/// job dir may be ignored), but ruinous when paired with a `.` path
/// against a developer worktree where `target/` is multi-gigabyte
/// build output.
pub async fn commit_stage_changes(
    ctx: &RunnerContext,
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: StageId,
    repo: &std::path::Path,
    subject: &str,
) -> Result<bool, codeless_adapters_host::GitCommitError> {
    emit_trio_started(ctx, store, task_id, stage_id, TodoKind::Git).await;
    let repo = repo.to_path_buf();
    let subject = subject.to_string();
    let join = tokio::task::spawn_blocking(move || {
        codeless_adapters_host::commit_all_changes(&repo, &subject)
    })
    .await;
    let result = match join {
        Ok(r) => r,
        Err(err) => Err(codeless_adapters_host::GitCommitError::Io {
            op: "join",
            source: std::io::Error::other(err.to_string()),
        }),
    };
    let (status, failure_detail) = match &result {
        Ok(true) => (TodoStatus::Done, None),
        Ok(false) => (TodoStatus::Skipped, None),
        Err(err) => (
            TodoStatus::Failed,
            Some(format!("git commit failed: {err}")),
        ),
    };
    emit_trio_completed(
        ctx,
        store,
        task_id,
        stage_id,
        TodoKind::Git,
        status,
        failure_detail,
    )
    .await;
    result
}

/// Belt-and-braces guard for the stage-completion trio gate. The
/// gate (`stage_trio_gate`) refuses to let a stage emit
/// `StageCompleted{ Passed }` until every closing-trio row is `Done`
/// or `Skipped`. Today only `claude_runner` drives the `Docs` rail,
/// and only `verify_runner` / `commit_stage_changes` drive `Checks`
/// and `Git` — a runner that doesn't go through that path (the mock
/// runner, a future `codex` / `anthropic` runner) leaves at least
/// one rail `Pending` forever and the stage hangs. Calling this
/// immediately before the gate flips every still-`Pending` trio row
/// to `Skipped` so the gate can resolve.
///
/// Rows already in a terminal state are left alone — `Done` stays
/// `Done`, a `Failed` row keeps its failure (and rightly keeps the
/// gate red so the runner's `RunnerOutcome::Failed` path takes
/// over). `InProgress` is left alone too: the caller that emitted
/// `InProgress` is the one obliged to publish the matching
/// terminal event, and stomping on it here would hide a runner
/// bug behind a phantom `Skipped`.
pub async fn skip_pending_trio_rows(
    ctx: &RunnerContext,
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: StageId,
) {
    let todos = match store.list_todos_for_task(task_id).await {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!(?err, %task_id, "skip_pending_trio_rows: list_todos failed");
            return;
        }
    };
    for kind in TodoKind::TRIO {
        let Some(row) = todos.iter().find(|t| t.kind == kind) else {
            continue;
        };
        if !matches!(row.status, TodoStatus::Pending) {
            continue;
        }
        publish(
            ctx,
            stage_id,
            task_id,
            Event::TodoCompleted {
                todo_id: row.id,
                status: TodoStatus::Skipped,
                failure_detail: None,
            },
        )
        .await;
    }
}

async fn publish(ctx: &RunnerContext, stage_id: StageId, task_id: TaskId, event: Event) {
    if let Err(err) = ctx
        .bus
        .publish(
            Some(ctx.job_id),
            Some(stage_id),
            Some(task_id),
            event,
            now_ms(),
        )
        .await
    {
        tracing::warn!(?err, "trio emitter: bus publish failed; continuing");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use codeless_types::{
        EventEnvelope, GitAuth, Job, JobId, JobStatus, Repo, RepoId, Stage, StageStatus, Task,
        TaskStatus, Todo, UnixMillis, WorkspaceMode,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio_stream::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::event_bus::{EventBus, SubscribeFilter};
    use crate::migrations::MIGRATOR;

    async fn seed_store_with_task() -> (Arc<SqliteStore>, TaskId, StageId) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = Arc::new(SqliteStore::new(pool));
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
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Running,
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
        (store, task.id, stage.id)
    }

    async fn fresh_bus() -> Arc<EventBus> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        Arc::new(EventBus::new(pool, 64))
    }

    fn ctx_with(bus: Arc<EventBus>) -> RunnerContext {
        RunnerContext {
            job_id: JobId::new(),
            stage_id: None,
            bus,
            worktree_path: None,
            cancel: CancellationToken::new(),
        }
    }

    fn trio_row(task_id: TaskId, ordinal: u32, kind: TodoKind) -> Todo {
        Todo {
            id: codeless_types::TodoId::new(),
            task_id,
            ordinal,
            title: format!("{kind:?}"),
            status: TodoStatus::Pending,
            kind,
            created_at: UnixMillis(0),
            started_at: None,
            ended_at: None,
            failure_detail: None,
        }
    }

    #[tokio::test]
    async fn find_trio_id_returns_row_with_matching_kind() {
        let (store, task_id, _) = seed_store_with_task().await;
        let docs = trio_row(task_id, 11, TodoKind::Docs);
        store.insert_todo(&docs).await.unwrap();
        let found = find_trio_id(&store, task_id, TodoKind::Docs).await;
        assert_eq!(found, Some(docs.id));
        // A kind that is not present resolves to `None`.
        assert!(find_trio_id(&store, task_id, TodoKind::Git).await.is_none());
    }

    #[tokio::test]
    async fn emit_trio_started_publishes_in_progress_event() {
        let (store, task_id, stage_id) = seed_store_with_task().await;
        let checks = trio_row(task_id, 10, TodoKind::Checks);
        store.insert_todo(&checks).await.unwrap();
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));

        emit_trio_started(&ctx, &store, task_id, stage_id, TodoKind::Checks).await;

        let env = tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
            .await
            .expect("event arrived")
            .expect("stream open")
            .unwrap();
        match env.event {
            Event::TodoUpdated { todo_id, status } => {
                assert_eq!(todo_id, checks.id);
                assert_eq!(status, TodoStatus::InProgress);
            }
            other => panic!("expected TodoUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_trio_completed_publishes_terminal_status() {
        let (store, task_id, stage_id) = seed_store_with_task().await;
        let git = trio_row(task_id, 12, TodoKind::Git);
        store.insert_todo(&git).await.unwrap();
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));

        emit_trio_completed(
            &ctx,
            &store,
            task_id,
            stage_id,
            TodoKind::Git,
            TodoStatus::Skipped,
            None,
        )
        .await;

        let env = tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
            .await
            .expect("event arrived")
            .expect("stream open")
            .unwrap();
        match env.event {
            Event::TodoCompleted {
                todo_id,
                status,
                failure_detail,
            } => {
                assert_eq!(todo_id, git.id);
                assert_eq!(status, TodoStatus::Skipped);
                assert!(failure_detail.is_none());
            }
            other => panic!("expected TodoCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_no_ops_when_trio_row_missing() {
        let (store, task_id, stage_id) = seed_store_with_task().await;
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));
        // No trio rows inserted; emit must publish nothing.
        emit_trio_started(&ctx, &store, task_id, stage_id, TodoKind::Docs).await;
        emit_trio_completed(
            &ctx,
            &store,
            task_id,
            stage_id,
            TodoKind::Docs,
            TodoStatus::Done,
            None,
        )
        .await;
        let next = tokio::time::timeout(std::time::Duration::from_millis(30), sub.next()).await;
        assert!(next.is_err(), "no events expected; got {next:?}");
    }

    fn init_git_repo(dir: &std::path::Path) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "test"][..],
            &["commit", "--allow-empty", "-q", "-m", "root"][..],
        ] {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args.iter().copied())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        }
    }

    #[tokio::test]
    async fn commit_stage_changes_emits_done_when_commit_produced() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        tokio::fs::write(&p, "hi").await.unwrap();

        let (store, task_id, stage_id) = seed_store_with_task().await;
        let git = trio_row(task_id, 12, TodoKind::Git);
        store.insert_todo(&git).await.unwrap();
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));

        let _ = p;
        let made = commit_stage_changes(&ctx, &store, task_id, stage_id, tmp.path(), "add")
            .await
            .unwrap();
        assert!(made, "first commit on a new file produces a commit");

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }
        assert!(got.iter().any(|e| matches!(
            e,
            Event::TodoUpdated {
                status: TodoStatus::InProgress,
                ..
            }
        )));
        assert!(got.iter().any(|e| matches!(
            e,
            Event::TodoCompleted {
                status: TodoStatus::Done,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn commit_stage_changes_emits_skipped_when_no_diff() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let p = tmp.path().join("hello.md");
        tokio::fs::write(&p, "hi").await.unwrap();
        // Pre-commit the file so the next commit_all_changes call
        // finds nothing to stage and returns Ok(false) — the
        // no-diff path documented as the trio's `Skipped` case.
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["add", "hello.md"])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["commit", "-q", "-m", "seed"])
            .status()
            .unwrap();

        let (store, task_id, stage_id) = seed_store_with_task().await;
        let git = trio_row(task_id, 12, TodoKind::Git);
        store.insert_todo(&git).await.unwrap();
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));

        let _ = p;
        let made = commit_stage_changes(&ctx, &store, task_id, stage_id, tmp.path(), "noop")
            .await
            .unwrap();
        assert!(!made, "no diff means no commit");

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }
        assert!(got.iter().any(|e| matches!(
            e,
            Event::TodoCompleted {
                status: TodoStatus::Skipped,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn skip_pending_trio_rows_only_skips_pending() {
        let (store, task_id, stage_id) = seed_store_with_task().await;
        // Mixed initial state: Checks Pending (should flip),
        // Docs InProgress (must NOT be touched — that runner still
        // owes a terminal event), Git Done (already terminal).
        let mut checks = trio_row(task_id, 10, TodoKind::Checks);
        let mut docs = trio_row(task_id, 11, TodoKind::Docs);
        let mut git = trio_row(task_id, 12, TodoKind::Git);
        docs.status = TodoStatus::InProgress;
        git.status = TodoStatus::Done;
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        // The recorder mirrors row status from the wire events; mirror
        // it here so the helper sees the same world.
        store
            .update_todo_status(docs.id, TodoStatus::InProgress, UnixMillis(1), None)
            .await
            .unwrap();
        store
            .update_todo_status(git.id, TodoStatus::Done, UnixMillis(2), None)
            .await
            .unwrap();
        // Refresh local handles so the assert below matches what the
        // store now has.
        checks.status = TodoStatus::Pending;
        let _ = (docs, git);

        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = ctx_with(Arc::clone(&bus));

        skip_pending_trio_rows(&ctx, &store, task_id, stage_id).await;

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }
        // Exactly one `TodoCompleted { Skipped }` for the Checks row;
        // the InProgress and Done rows produce no event.
        assert_eq!(got.len(), 1, "expected one event, got {got:?}");
        match &got[0] {
            Event::TodoCompleted {
                todo_id,
                status,
                failure_detail,
            } => {
                assert_eq!(*todo_id, checks.id);
                assert_eq!(*status, TodoStatus::Skipped);
                assert!(failure_detail.is_none());
            }
            other => panic!("expected TodoCompleted, got {other:?}"),
        }
    }
}
