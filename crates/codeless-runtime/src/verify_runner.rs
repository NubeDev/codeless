//! Per-stage verify gate runner. Iterates a stage's `Vec<VerifyStep>`,
//! runs each step's shell command in order, and emits one event per
//! step so the UI can render a glyph per gate rather than a single
//! pass/fail for the stage.
//!
//! Event contract (DOCS jobs-updates-1/stage-2-events.md):
//!
//! - `verify-step-started` is emitted once per step, in declaration
//!   order, **including** for steps that will end up skipped — the
//!   UI relies on a started/(passed|failed|skipped) pair per index.
//! - The first failing step is followed by `verify-step-failed`;
//!   every subsequent step in the list emits `verify-step-started`
//!   plus `verify-step-skipped { reason: "prior-gate-red" }` rather
//!   than being silently omitted. Operator-visibility wins over wire
//!   compactness (SCOPE.md hard rule #3).
//! - The outer `verify-started` / `verify-passed` / `verify-failed`
//!   stage envelopes are emitted by the caller (`TemplateRunner`),
//!   not by this module. Splitting that responsibility keeps this
//!   function pure: it owns the per-step stream and reports the
//!   stage-level pass/fail to the caller as a return value.
//!
//! Shell execution is injected via the `VerifyExec` trait so unit
//! tests can drive the exact event sequence without spawning real
//! processes. The production wiring uses a shell-spawning impl from
//! `codeless-adapters-host` — process spawn lives there per the
//! workspace's R1 dependency rule.

use std::path::Path;

use async_trait::async_trait;
use codeless_types::{Event, StageId, TaskId, TodoKind, TodoStatus};

use crate::runner::RunnerContext;
use crate::store::SqliteStore;
use crate::template::VerifyStep;
use crate::time::now_ms;
use crate::trio_emitter::{emit_trio_completed, emit_trio_started};

/// One step's terminal outcome from the shell. `duration_ms` is the
/// wall-clock cost of the spawn so the UI's per-gate row can surface
/// slow steps without a separate log fetch. `tail` is only meaningful
/// on failure and is bounded by the caller (~16 lines is the wire
/// convention).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyStepResult {
    pub exit_code: i32,
    pub duration_ms: u64,
    pub tail: String,
}

impl VerifyStepResult {
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }
}

/// Shell execution seam for `run_verify`. The single method takes the
/// resolved working directory (the stage's worktree) and the verbatim
/// shell command from the YAML, and returns the terminal result. Made
/// async so the production impl can stream-collect output without
/// blocking the runtime thread; the test impl just returns a canned
/// result.
#[async_trait]
pub trait VerifyExec: Send + Sync {
    async fn run(&self, cwd: Option<&Path>, command: &str) -> VerifyStepResult;
}

/// Drive the verify gate for one stage. Returns the index of the
/// first failing step on failure, or `None` if every step passed.
/// The caller uses the return to decide whether to bracket with
/// `verify-passed` or `verify-failed`.
pub async fn run_verify(
    ctx: &RunnerContext,
    task_id: TaskId,
    stage_id: StageId,
    steps: &[VerifyStep],
    exec: &dyn VerifyExec,
    store: Option<&SqliteStore>,
) -> VerifyOutcome {
    // Flip the runtime-injected `Checks` trio row. The store argument
    // is optional so legacy test harnesses (and any future caller that
    // wants to drive verify without a SQLite store wired in) can pass
    // `None` and get the old behaviour; production callers thread the
    // job's store through so the stage-completion gate has a row to
    // resolve.
    if let Some(store) = store {
        emit_trio_started(ctx, store, task_id, stage_id, TodoKind::Checks).await;
    }
    let mut failure: Option<(u32, i32)> = None;
    for (idx, step) in steps.iter().enumerate() {
        let step_index = idx as u32;
        publish(
            ctx,
            stage_id,
            task_id,
            Event::VerifyStepStarted {
                stage_id,
                step_index,
                name: step.name.clone(),
            },
        )
        .await;

        if failure.is_some() {
            // A prior step has already failed. Emit `skipped` instead
            // of running the command — surfacing the skip explicitly
            // is the operator-visibility contract; silent omission
            // would leave the UI's per-gate row blank.
            publish(
                ctx,
                stage_id,
                task_id,
                Event::VerifyStepSkipped {
                    stage_id,
                    step_index,
                    name: step.name.clone(),
                    reason: "prior-gate-red".to_string(),
                },
            )
            .await;
            continue;
        }

        let result = exec
            .run(ctx.worktree_path.as_deref(), step.run.as_str())
            .await;
        if result.passed() {
            publish(
                ctx,
                stage_id,
                task_id,
                Event::VerifyStepPassed {
                    stage_id,
                    step_index,
                    name: step.name.clone(),
                    duration_ms: result.duration_ms,
                },
            )
            .await;
        } else {
            failure = Some((step_index, result.exit_code));
            publish(
                ctx,
                stage_id,
                task_id,
                Event::VerifyStepFailed {
                    stage_id,
                    step_index,
                    name: step.name.clone(),
                    exit_code: result.exit_code,
                    tail: result.tail,
                },
            )
            .await;
        }
    }

    let outcome = match failure {
        Some((step_index, exit_code)) => VerifyOutcome::Failed {
            step_index,
            exit_code,
        },
        None => VerifyOutcome::Passed,
    };
    if let Some(store) = store {
        let (trio_status, failure_detail) = match &outcome {
            VerifyOutcome::Passed => (TodoStatus::Done, None),
            VerifyOutcome::Failed {
                step_index,
                exit_code,
            } => (
                TodoStatus::Failed,
                Some(format!("verify step {step_index} exited {exit_code}")),
            ),
        };
        emit_trio_completed(
            ctx,
            store,
            task_id,
            stage_id,
            TodoKind::Checks,
            trio_status,
            failure_detail,
        )
        .await;
    }
    outcome
}

/// Stage-level summary returned to the caller. The caller uses
/// `step_index` / `exit_code` from the `Failed` arm to build the
/// outer `verify-failed` envelope without re-walking the steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Passed,
    Failed { step_index: u32, exit_code: i32 },
}

/// Production `VerifyExec` wired to `codeless_adapters_host::run_shell`.
/// The trait can't be implemented inside the adapter crate (`VerifyExec`
/// is defined here; the dependency direction goes runtime → adapters,
/// not back) so the impl lives here and the actual process spawn
/// happens in the adapter — R1's "process spawn lives in
/// `codeless-adapters-host` only" stays intact. `run_shell` is
/// synchronous; the trait is async, so the call goes through
/// `spawn_blocking` to keep the reactor unblocked on a slow gate.
pub struct HostVerifyExec;

#[async_trait]
impl VerifyExec for HostVerifyExec {
    async fn run(&self, cwd: Option<&Path>, command: &str) -> VerifyStepResult {
        let cwd_owned = cwd.map(|p| p.to_path_buf());
        let cmd = command.to_string();
        let join = tokio::task::spawn_blocking(move || {
            codeless_adapters_host::run_shell(cwd_owned.as_deref(), &cmd)
        })
        .await;
        match join {
            Ok(out) => VerifyStepResult {
                exit_code: out.exit_code,
                duration_ms: out.duration_ms,
                tail: out.tail,
            },
            Err(err) => VerifyStepResult {
                exit_code: -1,
                duration_ms: 0,
                tail: format!("verify exec join failed: {err}"),
            },
        }
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
        tracing::warn!(?err, "verify runner: bus publish failed; continuing");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use codeless_types::{EventEnvelope, JobId};
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio_stream::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::event_bus::{EventBus, SubscribeFilter};
    use crate::migrations::MIGRATOR;
    use crate::runner::RunnerContext;

    struct CannedExec {
        results: Vec<VerifyStepResult>,
        cursor: tokio::sync::Mutex<usize>,
    }

    #[async_trait]
    impl VerifyExec for CannedExec {
        async fn run(&self, _cwd: Option<&Path>, _command: &str) -> VerifyStepResult {
            let mut c = self.cursor.lock().await;
            let r = self.results[*c].clone();
            *c += 1;
            r
        }
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

    fn step(name: &str) -> VerifyStep {
        VerifyStep {
            name: name.to_string(),
            run: format!("{name}-cmd"),
        }
    }

    #[tokio::test]
    async fn four_step_verify_with_step_three_failure_emits_full_sequence() {
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = RunnerContext {
            job_id: JobId::new(),
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let stage_id = StageId::new();
        let task_id = TaskId::new();

        let steps = vec![step("check"), step("test"), step("clippy"), step("fmt")];
        let exec = CannedExec {
            results: vec![
                VerifyStepResult {
                    exit_code: 0,
                    duration_ms: 10,
                    tail: String::new(),
                },
                VerifyStepResult {
                    exit_code: 0,
                    duration_ms: 12,
                    tail: String::new(),
                },
                VerifyStepResult {
                    exit_code: 7,
                    duration_ms: 9,
                    tail: "error: clippy failed".into(),
                },
            ],
            cursor: tokio::sync::Mutex::new(0),
        };

        let outcome = run_verify(&ctx, task_id, stage_id, &steps, &exec, None).await;
        assert_eq!(
            outcome,
            VerifyOutcome::Failed {
                step_index: 2,
                exit_code: 7,
            }
        );

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }

        let started_count = got
            .iter()
            .filter(|e| matches!(e, Event::VerifyStepStarted { .. }))
            .count();
        assert_eq!(started_count, 4, "every step emits started: {got:#?}");

        let passed_indices: Vec<u32> = got
            .iter()
            .filter_map(|e| match e {
                Event::VerifyStepPassed { step_index, .. } => Some(*step_index),
                _ => None,
            })
            .collect();
        assert_eq!(passed_indices, vec![0, 1]);

        let failed_indices: Vec<u32> = got
            .iter()
            .filter_map(|e| match e {
                Event::VerifyStepFailed { step_index, .. } => Some(*step_index),
                _ => None,
            })
            .collect();
        assert_eq!(failed_indices, vec![2]);

        let skipped: Vec<(u32, String)> = got
            .iter()
            .filter_map(|e| match e {
                Event::VerifyStepSkipped {
                    step_index, reason, ..
                } => Some((*step_index, reason.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, vec![(3, "prior-gate-red".to_string())]);

        // Ordering check: started(0..=3) precede their respective
        // terminal events; failed(2) precedes skipped(3).
        let pos = |needle: &Event| got.iter().position(|e| e == needle).unwrap();
        let started_2 = pos(&Event::VerifyStepStarted {
            stage_id,
            step_index: 2,
            name: "clippy".into(),
        });
        let failed_2 = pos(&Event::VerifyStepFailed {
            stage_id,
            step_index: 2,
            name: "clippy".into(),
            exit_code: 7,
            tail: "error: clippy failed".into(),
        });
        assert!(started_2 < failed_2);
        let started_3 = pos(&Event::VerifyStepStarted {
            stage_id,
            step_index: 3,
            name: "fmt".into(),
        });
        let skipped_3 = pos(&Event::VerifyStepSkipped {
            stage_id,
            step_index: 3,
            name: "fmt".into(),
            reason: "prior-gate-red".into(),
        });
        assert!(failed_2 < started_3);
        assert!(started_3 < skipped_3);
    }

    #[tokio::test]
    async fn all_steps_passing_returns_passed_with_no_skips() {
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = RunnerContext {
            job_id: JobId::new(),
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let stage_id = StageId::new();
        let task_id = TaskId::new();

        let steps = vec![step("check"), step("test")];
        let exec = CannedExec {
            results: vec![
                VerifyStepResult {
                    exit_code: 0,
                    duration_ms: 5,
                    tail: String::new(),
                },
                VerifyStepResult {
                    exit_code: 0,
                    duration_ms: 6,
                    tail: String::new(),
                },
            ],
            cursor: tokio::sync::Mutex::new(0),
        };

        assert_eq!(
            run_verify(&ctx, task_id, stage_id, &steps, &exec, None).await,
            VerifyOutcome::Passed
        );

        let mut events: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            events.push(event);
        }
        assert!(!events
            .iter()
            .any(|e| matches!(e, Event::VerifyStepSkipped { .. })));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::VerifyStepPassed { .. }))
                .count(),
            2,
        );
    }

    #[tokio::test]
    async fn passing_run_flips_checks_trio_to_done() {
        // With a `Some(store)` argument, `run_verify` must publish
        // `TodoUpdated(InProgress)` for the `Checks` trio row before
        // the first step and `TodoCompleted(Done)` after the run
        // succeeds. The recorder is the consumer in production; here
        // we drive the bus directly and assert the wire envelopes.
        use codeless_types::{
            GitAuth, Job, JobId, JobStatus, Repo, RepoId, Stage, StageStatus, Task, TaskStatus,
            Todo, TodoId, TodoKind, TodoStatus, UnixMillis, WorkspaceMode,
        };
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = RunnerContext {
            job_id: JobId::new(),
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let stage_id = StageId::new();
        let task_id = TaskId::new();

        // Seed the trio row through a dedicated store (sharing the
        // bus's pool would conflate event rows with store rows).
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = crate::store::SqliteStore::new(pool);
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
            id: ctx.job_id,
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
        store
            .insert_task_minimal(&Task {
                id: task_id,
                stage_id,
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
            })
            .await
            .unwrap();
        let checks_id = TodoId::new();
        store
            .insert_todo(&Todo {
                id: checks_id,
                task_id,
                ordinal: u32::MAX - 2,
                title: "checks".into(),
                status: TodoStatus::Pending,
                kind: TodoKind::Checks,
                created_at: UnixMillis(0),
                started_at: None,
                ended_at: None,
                failure_detail: None,
            })
            .await
            .unwrap();

        let steps = vec![step("check")];
        let exec = CannedExec {
            results: vec![VerifyStepResult {
                exit_code: 0,
                duration_ms: 5,
                tail: String::new(),
            }],
            cursor: tokio::sync::Mutex::new(0),
        };
        let outcome = run_verify(&ctx, task_id, stage_id, &steps, &exec, Some(&store)).await;
        assert_eq!(outcome, VerifyOutcome::Passed);

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }
        let started = got
            .iter()
            .position(|e| {
                matches!(e, Event::TodoUpdated { todo_id, status }
                if *todo_id == checks_id && *status == TodoStatus::InProgress)
            })
            .expect("InProgress event missing");
        let completed = got
            .iter()
            .position(|e| {
                matches!(e, Event::TodoCompleted { todo_id, status, .. }
                if *todo_id == checks_id && *status == TodoStatus::Done)
            })
            .expect("Done event missing");
        assert!(
            started < completed,
            "InProgress must precede Done; got {got:#?}"
        );
    }

    #[tokio::test]
    async fn failing_run_flips_checks_trio_to_failed() {
        // Mirror of the passing test: a step exit_code != 0 must end
        // the trio row in `Failed`, not `Done` — the stage-completion
        // gate would otherwise treat a red verify run as resolved.
        use codeless_types::{
            GitAuth, Job, JobId, JobStatus, Repo, RepoId, Stage, StageStatus, Task, TaskStatus,
            Todo, TodoId, TodoKind, TodoStatus, UnixMillis, WorkspaceMode,
        };
        let bus = fresh_bus().await;
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        let ctx = RunnerContext {
            job_id: JobId::new(),
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let stage_id = StageId::new();
        let task_id = TaskId::new();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = crate::store::SqliteStore::new(pool);
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
            id: ctx.job_id,
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
        store
            .insert_task_minimal(&Task {
                id: task_id,
                stage_id,
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
            })
            .await
            .unwrap();
        let checks_id = TodoId::new();
        store
            .insert_todo(&Todo {
                id: checks_id,
                task_id,
                ordinal: u32::MAX - 2,
                title: "checks".into(),
                status: TodoStatus::Pending,
                kind: TodoKind::Checks,
                created_at: UnixMillis(0),
                started_at: None,
                ended_at: None,
                failure_detail: None,
            })
            .await
            .unwrap();

        let steps = vec![step("check")];
        let exec = CannedExec {
            results: vec![VerifyStepResult {
                exit_code: 1,
                duration_ms: 5,
                tail: "boom".into(),
            }],
            cursor: tokio::sync::Mutex::new(0),
        };
        let outcome = run_verify(&ctx, task_id, stage_id, &steps, &exec, Some(&store)).await;
        assert!(matches!(outcome, VerifyOutcome::Failed { .. }));

        let mut got: Vec<Event> = Vec::new();
        while let Some(Ok(EventEnvelope { event, .. })) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next())
                .await
                .ok()
                .flatten()
        {
            got.push(event);
        }
        assert!(got.iter().any(
            |e| matches!(e, Event::TodoCompleted { todo_id, status, failure_detail }
            if *todo_id == checks_id
                && *status == TodoStatus::Failed
                && failure_detail.as_deref().is_some_and(|s| s.contains("verify step")))
        ));
    }
}
