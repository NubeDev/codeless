use codeless_types::{JobStatus, StageStatus, TaskId, TaskStatus};
use thiserror::Error;

use crate::store::SqliteStore;

/// Refused transition. The driver treats this as a programming error
/// (the state machine guarded against an illegal move), not a user
/// error — surface paths convert it to `RpcError::Conflict` when the
/// runtime is asked to act on a job that is already terminal.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("illegal transition: {kind} {from:?} -> {to:?}")]
pub struct TransitionError {
    pub kind: &'static str,
    pub from: String,
    pub to: String,
}

fn deny<F: std::fmt::Debug, T: std::fmt::Debug>(
    kind: &'static str,
    from: F,
    to: T,
) -> TransitionError {
    TransitionError {
        kind,
        from: format!("{from:?}"),
        to: format!("{to:?}"),
    }
}

/// Job lifecycle guard. Allowed edges:
/// - `Queued -> Running | Stopped`
/// - `Draft -> Queued | Stopped`
/// - `Queued -> Running | Stopped`
/// - `Running -> AwaitingReview | Completed | Failed | Stopped`
/// - `AwaitingReview -> Running | Completed | Stopped`
///
/// `Draft` is the editable pre-run holding state — the user has
/// submitted the job but not asked the driver to run it yet.
/// Terminal states (`Completed`, `Failed`, `Stopped`) accept no further
/// transitions; the runtime reports `Conflict` when asked. This matches
/// the lifecycle described in SCOPE.md "Runtime / scheduler".
pub fn transition_job(from: JobStatus, to: JobStatus) -> Result<(), TransitionError> {
    use JobStatus::*;
    let ok = matches!(
        (from, to),
        (Draft, Queued)
            | (Draft, Stopped)
            | (Queued, Running)
            | (Queued, Stopped)
            // Driver give-up edge. Used by `job_driver_loop` when
            // `drive_job` keeps erroring before the row ever reaches
            // Running — the retry budget is exhausted (retryable
            // failures) or the error is unrecoverable (runner not
            // enabled, template parse). `stop_reason = RunnerCrash`
            // is recorded on the row so the UI distinguishes this
            // from a clean user-driven Failed state.
            | (Queued, Failed)
            | (Running, AwaitingReview)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Stopped)
            | (AwaitingReview, Running)
            | (AwaitingReview, Completed)
            | (AwaitingReview, Stopped)
            // Resume paths (A0 — intra-stage session continuation per
            // SCOPE.md hard rule #1). A user-resumed job re-enters the
            // queue with its captured `Stage.session_id` intact; the
            // claude adapter passes that id via `CliCfg::resume_id` so
            // the agent picks up the same conversation rather than
            // re-deriving from scratch. `Failed` is included so a
            // cost-cap that fired mid-tool-call (sometimes recorded as
            // failed rather than stopped depending on cancel ordering)
            // is also resumable.
            | (Stopped, Queued)
            | (Failed, Queued)
            // Pause paths (user-initiated `pause_job`, or cap-watcher
            // pausing instead of stopping when the stage has a
            // captured `Stage.session_id`). A paused job is expected
            // to be resumed; it is NOT a terminal state. Resume goes
            // back through `Queued` so the driver re-picks it up via
            // the same loop as a fresh submit.
            | (Running, Paused)
            | (AwaitingReview, Paused)
            | (Paused, Queued)
            | (Paused, Stopped)
            // Reset paths (user-driven `reset_job` recovery hatch).
            // Stuck states (`Queued` the driver could not move,
            // `Failed`, `Stopped`) collapse back to `Draft` so the
            // operator can edit the spec or re-`start_job` without
            // the resume cap dance. `Running`, `Paused`, and
            // `AwaitingReview` are deliberately excluded — those are
            // not stuck-states; they go through `stop_job` /
            // `pause_job` first.
            | (Queued, Draft)
            | (Failed, Draft)
            | (Stopped, Draft),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("job", from, to))
    }
}

/// Stage lifecycle guard. Mirrors `transition_job` for the per-stage
/// state column. `AwaitingReview` is the verify-gated pause; a human (or
/// auto-approver) sends it to `Passed` or back to `Running`.
pub fn transition_stage(from: StageStatus, to: StageStatus) -> Result<(), TransitionError> {
    use StageStatus::*;
    let ok = matches!(
        (from, to),
        (Pending, Running)
            | (Running, AwaitingReview)
            | (Running, Passed)
            | (Running, Failed)
            | (AwaitingReview, Running)
            | (AwaitingReview, Passed)
            | (AwaitingReview, Failed),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("stage", from, to))
    }
}

/// Task lifecycle guard. `Enqueued -> Running -> Completed | Failed`,
/// with `Cancelled` reachable from any non-terminal state when the job
/// is stopped mid-flight.
pub fn transition_task(from: TaskStatus, to: TaskStatus) -> Result<(), TransitionError> {
    use TaskStatus::*;
    let ok = matches!(
        (from, to),
        (Enqueued, Running)
            | (Enqueued, Cancelled)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Cancelled),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("task", from, to))
    }
}

/// Closing-trio gate on `Running -> Passed`. The runtime injects three
/// trio rows (`Checks`, `Docs`, `Git`) at stage entry; the stage cannot
/// emit `Event::StageCompleted{ status: Passed }` until the trio
/// resolves. `Resolved` releases the stage; `Pending` keeps it open
/// (the caller retries with delay); `Failed { failures }` routes the
/// stage through the auto-bypass-eligible failure path with the
/// per-rail reason surfaced into the stage's `failure_detail`. The
/// transition functions above are pure status guards; resolving the
/// trio requires the persistent state machine (SQLite rows the
/// recorder owns), so the gate lives here as the async counterpart
/// to `transition_stage`.
pub async fn stage_trio_gate(
    store: &SqliteStore,
    terminal_task_id: TaskId,
) -> sqlx::Result<crate::store::TrioGateOutcome> {
    store.trio_gate_outcome(terminal_task_id).await
}

pub fn is_terminal_job(s: JobStatus) -> bool {
    matches!(
        s,
        JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::JobStatus;

    #[test]
    fn reset_allows_queued_to_draft() {
        transition_job(JobStatus::Queued, JobStatus::Draft).expect("queued -> draft is allowed");
    }

    #[test]
    fn reset_allows_failed_to_draft() {
        transition_job(JobStatus::Failed, JobStatus::Draft).expect("failed -> draft is allowed");
    }

    #[test]
    fn reset_allows_stopped_to_draft() {
        transition_job(JobStatus::Stopped, JobStatus::Draft).expect("stopped -> draft is allowed");
    }

    #[test]
    fn reset_refuses_running_to_draft() {
        let err = transition_job(JobStatus::Running, JobStatus::Draft)
            .expect_err("running -> draft must be refused");
        assert_eq!(err.kind, "job");
    }

    #[tokio::test]
    async fn trio_gate_blocks_until_all_three_resolved() {
        // The gate is the async counterpart to `transition_stage`: a
        // pure status guard cannot tell whether the trio rows have
        // landed, so the gate consults the store. This test pins the
        // "blocked until all three resolved" contract end-to-end so a
        // future refactor that moves the check out of state_machine
        // cannot drop the gate semantics.
        use crate::migrations::MIGRATOR;
        use codeless_types::{
            CostCents, GitAuth, Job, JobId, RepoId, Stage, StageId, Task, TaskStatus, Todo, TodoId,
            TodoKind, TodoStatus, UnixMillis, WorkspaceMode,
        };
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool);

        let repo = codeless_types::Repo {
            id: RepoId::new(),
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/k".into(),
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
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
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
            cost_cents: CostCents(0),
            input_tokens: 0,
            output_tokens: 0,
            started_at: None,
            ended_at: None,
        };
        store.insert_task_minimal(&task).await.unwrap();

        // No trio rows yet — gate blocks.
        assert_eq!(
            stage_trio_gate(&store, task.id).await.unwrap(),
            crate::store::TrioGateOutcome::Pending
        );

        let mk = |ord: u32, kind: TodoKind| Todo {
            id: TodoId::new(),
            task_id: task.id,
            ordinal: ord,
            title: "t".into(),
            status: TodoStatus::Pending,
            kind,
            created_at: UnixMillis(0),
            started_at: None,
            ended_at: None,
            failure_detail: None,
        };
        let checks = mk(10, TodoKind::Checks);
        let docs = mk(11, TodoKind::Docs);
        let git = mk(12, TodoKind::Git);
        for t in [&checks, &docs, &git] {
            store.insert_todo(t).await.unwrap();
        }
        // Rows exist but Pending — gate blocks.
        assert_eq!(
            stage_trio_gate(&store, task.id).await.unwrap(),
            crate::store::TrioGateOutcome::Pending
        );

        store
            .update_todo_status(checks.id, TodoStatus::Done, UnixMillis(1), None)
            .await
            .unwrap();
        store
            .update_todo_status(docs.id, TodoStatus::Done, UnixMillis(2), None)
            .await
            .unwrap();
        // Two of three — gate blocks.
        assert_eq!(
            stage_trio_gate(&store, task.id).await.unwrap(),
            crate::store::TrioGateOutcome::Pending
        );

        // `Skipped` counts as resolved (the no-diff git case).
        store
            .update_todo_status(git.id, TodoStatus::Skipped, UnixMillis(3), None)
            .await
            .unwrap();
        assert_eq!(
            stage_trio_gate(&store, task.id).await.unwrap(),
            crate::store::TrioGateOutcome::Resolved
        );
    }

    #[test]
    fn reset_refuses_paused_and_awaiting_review_to_draft() {
        assert!(transition_job(JobStatus::Paused, JobStatus::Draft).is_err());
        assert!(transition_job(JobStatus::AwaitingReview, JobStatus::Draft).is_err());
        assert!(transition_job(JobStatus::Completed, JobStatus::Draft).is_err());
    }
}
