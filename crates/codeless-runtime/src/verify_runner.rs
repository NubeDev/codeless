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
use codeless_types::{Event, StageId, TaskId};

use crate::runner::RunnerContext;
use crate::template::VerifyStep;
use crate::time::now_ms;

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
) -> VerifyOutcome {
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

    match failure {
        Some((step_index, exit_code)) => VerifyOutcome::Failed {
            step_index,
            exit_code,
        },
        None => VerifyOutcome::Passed,
    }
}

/// Stage-level summary returned to the caller. The caller uses
/// `step_index` / `exit_code` from the `Failed` arm to build the
/// outer `verify-failed` envelope without re-walking the steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Passed,
    Failed { step_index: u32, exit_code: i32 },
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

        let outcome = run_verify(&ctx, task_id, stage_id, &steps, &exec).await;
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
            run_verify(&ctx, task_id, stage_id, &steps, &exec).await,
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
}
