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
    ///
    /// `worktree` is the provisioned `git worktree` checkout (when
    /// available) — `.codeless/jobs/<name>/` lives inside it, so we
    /// resolve per-stage docs there rather than from the source repo.
    /// `None` (test harness path) skips doc resolution entirely.
    fn stage_prompt(
        &self,
        planned: PlannedStage<'_>,
        total: usize,
        worktree: Option<&std::path::Path>,
    ) -> String {
        let stage_num = planned.index + 1;
        let review_note = if planned.is_review {
            "\n\nThis is a REVIEW stage. The user will approve the work \
             you do here before the next stage runs (today the runtime \
             does not yet block on the gate, but emit the handover as \
             if review is the terminator)."
        } else {
            ""
        };

        // Per-stage docs: appended *after* global docs (which the
        // job_driver_loop already prepended to `job.prompt` once at
        // dispatch time). The structure mirrors that block so the
        // model sees `# Job docs` with stage-specific sections under
        // the same heading the first time it appeared.
        let stage_docs = match worktree {
            Some(wt) if !planned.docs.is_empty() => {
                crate::job_dir::read_docs_ordered(wt, &self.template.name, planned.docs)
            }
            _ => String::new(),
        };
        let stage_docs_block = if stage_docs.is_empty() {
            String::new()
        } else {
            // Rename the heading so a downstream reader can tell global
            // and per-stage blocks apart in the same prompt.
            stage_docs.replacen("# Job docs", &format!("# Stage {stage_num} docs"), 1) + "\n"
        };

        format!(
            "{stage_docs_block}\
             # Job goal\n\n{}\n\n\
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
                let prompt = self.stage_prompt(*stage, total, ctx.worktree_path.as_deref());
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
        // Parse each raw stage through the YAML round-trip so the
        // `REVIEW ` prefix on flat strings is honored consistently
        // with end-user templates.
        let stage_yaml = stages
            .iter()
            .map(|s| format!("  - {s}"))
            .collect::<Vec<_>>()
            .join("\n");
        let yaml = format!("name: t\ngoal: test goal\nstages:\n{stage_yaml}\n");
        JobTemplate::parse_yaml(&yaml).expect("template fixture parses")
    }

    #[test]
    fn stage_prompt_includes_goal_and_position() {
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[1], 2, None);
        assert!(prompt.contains("Stage 2 of 2"));
        assert!(prompt.contains("two"));
        assert!(prompt.contains("test goal"));
        assert!(!prompt.contains("REVIEW"));
    }

    #[test]
    fn review_prompt_carries_gate_note() {
        let r = TemplateRunner::new(template_with_stages(&["REVIEW gate", "after"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[0], 2, None);
        assert!(prompt.contains("REVIEW stage"));
    }

    #[test]
    fn stage_prompt_appends_per_stage_docs_when_worktree_resolves() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".codeless/jobs/webserver");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("routing.md"), "ROUTING DOC BODY").unwrap();
        fs::write(dir.join("handlers.md"), "HANDLERS DOC BODY").unwrap();

        let src = r#"
name: webserver
goal: Build server
stages:
  - title: scaffold
    docs:
      - routing.md
  - title: add handlers
    docs:
      - handlers.md
"#;
        let template = JobTemplate::parse_yaml(src).unwrap();
        let r = TemplateRunner::new(template);
        let planned = r.template.planned_stages();

        // Stage 1 sees routing.md, not handlers.md.
        let p1 = r.stage_prompt(planned[0], 2, Some(tmp.path()));
        assert!(
            p1.contains("# Stage 1 docs"),
            "missing stage-docs heading: {p1}"
        );
        assert!(p1.contains("ROUTING DOC BODY"));
        assert!(!p1.contains("HANDLERS DOC BODY"));

        // Stage 2 sees handlers.md, not routing.md.
        let p2 = r.stage_prompt(planned[1], 2, Some(tmp.path()));
        assert!(p2.contains("# Stage 2 docs"));
        assert!(p2.contains("HANDLERS DOC BODY"));
        assert!(!p2.contains("ROUTING DOC BODY"));
    }

    #[test]
    fn stage_prompt_omits_docs_block_when_stage_has_none() {
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[0], 2, None);
        assert!(!prompt.contains("# Stage 1 docs"));
        assert!(!prompt.contains("# Job docs"));
    }
}
