//! Multi-stage runner — turns a `.codeless/jobs/<name>.yaml` template
//! into a sequence of claude invocations, one per stage, with
//! `stage-started` / `stage-completed` envelopes around each.
//!
//! This is the first runner in the codebase that emits *user-authored*
//! stages (the YAML's `stages:` list) rather than runner-emitted
//! sub-events. The UI's `StageTree` finally has something real to
//! render: the user sees their planned stages tick off live.
//!
//! Scope gaps, documented honestly:
//!
//! - `REVIEW`-prefixed stages emit a `review-requested` envelope but do
//!   not block. The runtime's `approve_review` RPC exists; what's
//!   missing is the orchestrator wait — a `tokio::sync::Notify` keyed
//!   to the review row. Until that lands, REVIEW stages render in the
//!   timeline as "human gate noted" and the loop continues. The author
//!   of the template can still tell the next session "this stage
//!   should have been reviewed" from the log.
//! - `verify:` shell command (JOB-MODEL.md "one shell command, must
//!   exit 0") is not run between stages. The model is asked to commit
//!   per stage; if a stage's output is wrong, it carries forward.
//!   Real verify lands when there's a stage-runner harness to invoke.
//! - Cost / wall-clock caps are tracked at the JOB level, not the
//!   stage level. A single runaway stage can still hit the per-job
//!   cap; per-stage budgeting is a future refinement.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_types::{Event, ReviewId, StageId, StageStatus, TaskId};
use tokio_util::sync::CancellationToken;

use crate::claude_runner::ClaudeRunnerAdapter;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::template::{JobTemplate, PlannedStage};
use crate::time::now_ms;

/// Iterate the template's stages and run claude per stage. Each
/// stage gets its own `task_id` because every stage is "one
/// independent attempt at making the project state advance" — and
/// the AI bridge keys tool-calls / tokens by `task_id`. Sharing one
/// task_id across stages would collapse every per-stage assistant
/// message into the same bubble in the UI.
pub struct TemplateRunner {
    pub template: JobTemplate,
    /// Optional system-prompt override; passed through to each
    /// per-stage `ClaudeRunnerAdapter`. `None` keeps the headless
    /// default.
    pub system_prompt: Option<String>,
}

impl TemplateRunner {
    pub fn new(template: JobTemplate) -> Self {
        Self {
            template,
            system_prompt: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        let s = prompt.into();
        self.system_prompt = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Build the per-stage prompt the inner adapter sees. Carries the
    /// job-wide goal AND the stage title so the model never loses the
    /// big picture between stages. Numbered position is included so a
    /// stage prompt mentioning "the next one" can be interpreted.
    fn stage_prompt(&self, planned: PlannedStage<'_>, total: usize) -> String {
        let stage_num = planned.index + 1;
        let review_note = if planned.is_review {
            "\n\nThis is a REVIEW stage. The user will approve the work \
             you do here before the next stage runs (today the runtime \
             does not yet block on the gate, but emit the handover as \
             if review is the terminator)."
        } else {
            ""
        };
        format!(
            "# Job goal\n\n{}\n\n\
             # Stage {stage_num} of {total}\n\n{}\n\
             \n\
             # What to do now\n\n\
             Implement only this stage. Commit your work with a message \
             starting with the stage title. Do not start the next stage; \
             a fresh session will pick it up.{review_note}\n",
            self.template.goal, planned.title,
        )
    }
}

#[async_trait]
impl Runner for TemplateRunner {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let planned = self.template.planned_stages();
        let total = planned.len();
        for stage in &planned {
            if ctx.cancel.is_cancelled() {
                tracing::info!(
                    stage = stage.title,
                    "template runner: cancelled before stage"
                );
                return RunnerOutcome::Failed {
                    reason: "cancelled".into(),
                };
            }
            let stage_id = StageId::new();
            let task_id = TaskId::new();
            // Emit stage-started so the UI's StageTree picks up this
            // user-authored stage in real time.
            publish(
                &ctx,
                stage_id,
                task_id,
                Event::StageStarted {
                    stage_id,
                    job_id: ctx.job_id,
                },
            )
            .await;

            if stage.is_review {
                // REVIEW stage: surface the gate as a review-requested
                // event but do not wait. The review row creation lives
                // on the runtime's review RPC surface; emitting the
                // event here keeps the timeline honest about the gate
                // without forcing the orchestrator to spin on a
                // notification channel that does not yet exist.
                let review_id = ReviewId::new();
                publish(
                    &ctx,
                    stage_id,
                    task_id,
                    Event::ReviewRequested {
                        review_id,
                        stage_id,
                    },
                )
                .await;
            } else {
                let prompt = self.stage_prompt(*stage, total);
                let mut adapter = ClaudeRunnerAdapter::new(prompt, task_id);
                if let Some(sp) = &self.system_prompt {
                    adapter = adapter.with_system_prompt(sp.clone());
                }
                let sub_ctx = RunnerContext {
                    job_id: ctx.job_id,
                    bus: Arc::clone(&ctx.bus),
                    worktree_path: ctx.worktree_path.clone(),
                    cancel: derive_cancel(&ctx.cancel),
                };
                match adapter.run(sub_ctx).await {
                    RunnerOutcome::Completed => {}
                    RunnerOutcome::Failed { reason } => {
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::StageCompleted {
                                stage_id,
                                status: StageStatus::Failed,
                            },
                        )
                        .await;
                        tracing::warn!(stage = stage.title, %reason, "stage failed; aborting template run");
                        return RunnerOutcome::Failed { reason };
                    }
                }
            }

            publish(
                &ctx,
                stage_id,
                task_id,
                Event::StageCompleted {
                    stage_id,
                    status: StageStatus::Passed,
                },
            )
            .await;
        }
        RunnerOutcome::Completed
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
        tracing::warn!(?err, "template runner: bus publish failed; continuing");
    }
}

fn derive_cancel(parent: &CancellationToken) -> CancellationToken {
    // Per-stage cancellation token that is cancelled when the parent
    // (driver-owned) is cancelled, but not vice-versa: aborting one
    // stage does not abort the entire job. Today this is academic
    // because each stage is awaited sequentially before the next, but
    // it keeps the seam clean for when per-stage retries land.
    let child = CancellationToken::new();
    let parent = parent.clone();
    let child_clone = child.clone();
    tokio::spawn(async move {
        parent.cancelled().await;
        child_clone.cancel();
    });
    child
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_with_stages(stages: &[&str]) -> JobTemplate {
        JobTemplate {
            name: "t".into(),
            goal: "test goal".into(),
            stages: stages.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn stage_prompt_includes_goal_and_position() {
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[1], 2);
        assert!(prompt.contains("Stage 2 of 2"));
        assert!(prompt.contains("two"));
        assert!(prompt.contains("test goal"));
        assert!(!prompt.contains("REVIEW"));
    }

    #[test]
    fn review_prompt_carries_gate_note() {
        let r = TemplateRunner::new(template_with_stages(&["REVIEW gate", "after"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[0], 2);
        assert!(prompt.contains("REVIEW stage"));
    }
}
