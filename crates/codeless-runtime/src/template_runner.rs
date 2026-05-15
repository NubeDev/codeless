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
//! - `verify:` shell command (JOB-MODEL.md "one shell command, must
//!   exit 0") is not run between stages. The model is asked to commit
//!   per stage; if a stage's output is wrong, it carries forward.
//!   Real verify lands when there's a stage-runner harness to invoke.
//! - Cost / wall-clock caps are tracked at the JOB level, not the
//!   stage level. A single runaway stage can still hit the per-job
//!   cap; per-stage budgeting is a future refinement.
//!
//! REVIEW stage semantics (the SESSION-MUTABLE-SCOPE Step 1 + 2
//! contract):
//!
//! A stage marked `review: true` (or with the `REVIEW ` flat-string
//! prefix) is a **model-driven blocking gate**, not a human-review
//! pause. Before the inner adapter is spawned, the runner runs a
//! deterministic Layer-1 **diff-verify pre-check** (Step 2): it reads
//! the *prior* stage's handover, extracts every path-shaped token
//! from its `Done` section, and confirms each one appears in the
//! worktree's `git diff`. A handover that claims paths the commit did
//! not touch is auto-FAIL with no model invoked — the cheapest,
//! highest-signal check in the ramp. If the pre-check passes, the
//! adapter runs with a prompt that instructs the model to emit a
//! single `PASS:` or `FAIL:` sentinel line in its handover. After the
//! adapter finishes, the template runner reads the handover file,
//! parses the sentinel via `review_gate::parse_review_verdict`, and:
//!
//! - `Pass` ⇒ the stage finishes `Passed` and the next stage runs.
//! - `Fail` ⇒ the stage finishes `Failed` and `run` returns
//!   `RunnerOutcome::Failed`; no later stages execute.
//! - Missing or ambiguous sentinel ⇒ same as `Fail`. A silent gate is
//!   treated as failure, so a model that forgets the sentinel can
//!   never accidentally wave a bad change through.
//!
//! No `Event::ReviewRequested` is emitted for these gates. That event
//! continues to mean "a human is asked to weigh in" (per
//! `SESSION-MUTABLE-SCOPE-DECISIONS.md`'s `ReviewRequested` vs
//! `ReviewGate*` decision); the model-driven gate's verdict surfaces
//! through the existing `StageCompleted` + `TaskCompleted` events.
//! Step 4 of the ramp adds patch-proposal emission on PASS; Step 5
//! enforces patch shape. This module is Step 1: sentinel parsing,
//! nothing more.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_types::{Event, ReviewId, StageId, StageStatus, TaskId};
use tokio_util::sync::CancellationToken;

use crate::claude_runner::ClaudeRunnerAdapter;
use crate::diff_verify::{
    fail_reason as diff_verify_fail_reason, verify_handover, DiffVerifyOutcome,
};
use crate::handover::handover_path;
use crate::review_gate::{parse_review_verdict, ReviewVerdict, VerdictParseError};
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::scope_patch_emit::{emit_from_handover, EmitOutcome};
use crate::store::SqliteStore;
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
    /// When `true`, each stage runs `MockRunner` instead of
    /// `ClaudeRunnerAdapter`. Used by `--enable-claude=false` so the
    /// iterate-loop UI (stage events, recorder, Spec pane) is
    /// drivable without a real claude install. Mock stages still
    /// emit `StageStarted` / `StageCompleted` / one `AiMessageComplete`
    /// per stage, so the StageRecorder records timing + cost (cost is
    /// 0 because mock doesn't bill anything).
    pub use_mock_runner: bool,
    /// Store handle for resume-aware stage execution. When a stage
    /// row already has `session_id: Some(...)` (because a previous
    /// run captured it before being interrupted by a cost-cap /
    /// user-stop / crash), the inner `ClaudeRunnerAdapter` receives
    /// it as `resume_id` and the claude wrapper passes `--continue`
    /// so the agent picks up the same conversation. `None` (test
    /// harness path) opts out: every stage runs fresh.
    pub store: Option<Arc<SqliteStore>>,
}

impl TemplateRunner {
    pub fn new(template: JobTemplate) -> Self {
        Self {
            template,
            system_prompt: None,
            use_mock_runner: false,
            store: None,
        }
    }

    /// Inject the store handle so the runner can look up each
    /// stage's captured `session_id` before invoking the inner
    /// adapter. Required for A0 (intra-stage session continuation);
    /// the test harnesses leave it unset and stages run fresh.
    pub fn with_store(mut self, store: Arc<SqliteStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        let s = prompt.into();
        self.system_prompt = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Opt into per-stage `MockRunner` for development / demos.
    pub fn with_mock_runner(mut self) -> Self {
        self.use_mock_runner = true;
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
        // REVIEW stages get an explicit sentinel contract appended to
        // the prompt. The runtime parses `PASS:` / `FAIL:` from the
        // handover after the stage runs; a missing or ambiguous
        // sentinel is treated as failure, so the wording has to be
        // unambiguous about what is required.
        let review_note = if planned.is_review {
            "\n\nThis is a REVIEW stage — a blocking gate, not a human \
             pause. Examine the diff from the prior WORK stages and \
             decide whether the rulebook's Layer-1 invariants hold \
             (R1 crate dependency direction, R2 single transport, \
             R4/R5 trust boundary, wire-formats untouched). Emit \
             exactly one sentinel line in your handover: `PASS: <one-\
             sentence reason>` if the gate holds, or `FAIL: <one-\
             sentence reason>` if it does not. The runtime parses the \
             sentinel and halts the job on FAIL. Do not propose patches \
             yet; that lands in a later ramp step."
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
        // Tracks the stage_id of the most recent stage that finished
        // (Passed) so a REVIEW stage's diff-verify pre-check can read
        // the prior WORK stage's handover by key rather than rely on
        // mtime-ranked discovery. Set after each stage's StageCompleted
        // emit; consumed at the top of the next REVIEW iteration.
        let mut prev_stage_id: Option<StageId> = None;
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
            // Carry `ordinal` (0-based, matches the YAML's stage
            // index) and `name` (the verbatim stage title, REVIEW
            // prefix included) so the StageRecorder can persist the
            // row without re-parsing the template.
            let name_for_event = if stage.is_review {
                format!("REVIEW {}", stage.title)
            } else {
                stage.title.to_owned()
            };
            publish(
                &ctx,
                stage_id,
                task_id,
                Event::StageStarted {
                    stage_id,
                    job_id: ctx.job_id,
                    ordinal: stage.index as u32,
                    name: name_for_event,
                },
            )
            .await;

            // SESSION-MUTABLE-SCOPE Step 2: Layer-1 diff-verify
            // pre-check. For REVIEW stages, walk every path-shaped
            // token in the *prior* stage's handover `Done` and confirm
            // each one appears in the worktree's git diff. A miss is
            // auto-FAIL with no model invoked — the highest-signal
            // check in the ramp, run before any tokens are spent. The
            // check is silently skipped when there is no prior stage
            // (a job that opens with REVIEW, or the mock-runner path
            // where the prior stage wrote no handover): the contract
            // is "verify the handover that *is* there," not "demand
            // one exist."
            if stage.is_review {
                if let Some(prev) = prev_stage_id {
                    if let Some(wt) = ctx.worktree_path.as_deref() {
                        match run_diff_verify_precheck(wt, ctx.job_id, prev).await {
                            PreCheckOutcome::Pass | PreCheckOutcome::Skipped => {}
                            PreCheckOutcome::Fail(reason) => {
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
                                tracing::warn!(
                                    stage = stage.title,
                                    %reason,
                                    "diff-verify pre-check failed; review stage auto-failed without invoking model"
                                );
                                return RunnerOutcome::Failed {
                                    reason: format!("diff-verify pre-check failed: {reason}"),
                                };
                            }
                        }
                    }
                }
            }

            // REVIEW and WORK stages share the same execution shape:
            // build the prompt, hand it to the inner adapter (or the
            // mock event sequence), then on success let control fall
            // through to the post-stage REVIEW sentinel check below.
            // Splitting them previously short-circuited the model
            // invocation for REVIEW stages, which made the gate a
            // theatre rather than a real check.
            {
                let prompt = self.stage_prompt(*stage, total, ctx.worktree_path.as_deref());
                let sub_ctx = RunnerContext {
                    job_id: ctx.job_id,
                    // Tag the per-stage child context with the stage id
                    // so the inner adapter can publish stage-scoped
                    // events (e.g. `StageSessionCaptured`) against the
                    // row TemplateRunner just opened.
                    stage_id: Some(stage_id),
                    bus: Arc::clone(&ctx.bus),
                    worktree_path: ctx.worktree_path.clone(),
                    cancel: derive_cancel(&ctx.cancel),
                };
                let outcome = if self.use_mock_runner {
                    // Mock stage: a small, realistic-looking event
                    // sequence so the recorder + UI see the same
                    // shape they'd see from claude. Cost is a small
                    // synthetic number per stage (1-3 cents) so the
                    // demo shows real rollup math without billing.
                    //
                    // Emit directly with the stage_id correlation
                    // baked into the envelope (MockRunner publishes
                    // events with stage_id=None, which would break the
                    // recorder's per-stage attribution).
                    let synth_cost = ((stage.index as i64) % 3) + 1;
                    publish(&ctx, stage_id, task_id, Event::TaskStarted { task_id }).await;
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    publish(
                        &ctx,
                        stage_id,
                        task_id,
                        Event::AiToken {
                            task_id,
                            delta: format!("mock: working on '{}'\n", stage.title),
                        },
                    )
                    .await;
                    publish(
                        &ctx,
                        stage_id,
                        task_id,
                        Event::AiMessageComplete {
                            task_id,
                            input_tokens: 128,
                            output_tokens: 64,
                            cost_cents: codeless_types::CostCents(synth_cost),
                        },
                    )
                    .await;
                    publish(
                        &ctx,
                        stage_id,
                        task_id,
                        Event::TaskCompleted {
                            task_id,
                            status: codeless_types::TaskStatus::Completed,
                        },
                    )
                    .await;
                    // Hold ctx to silence unused warnings around the
                    // branch's `sub_ctx`.
                    drop(sub_ctx);
                    // The real claude adapter writes the handover; the
                    // mock branch doesn't go through it, so synthesise
                    // a handover for REVIEW stages with the required
                    // PASS sentinel — otherwise the gate downstream
                    // would fail every mock REVIEW run. The synthetic
                    // verdict is intentionally PASS: mock mode is for
                    // UI / event-shape demos, not for exercising the
                    // gate's FAIL path (production REVIEW with claude
                    // exercises FAIL via real model output).
                    if stage.is_review {
                        if let Some(wt) = ctx.worktree_path.as_ref() {
                            write_mock_review_handover(wt, ctx.job_id, stage_id).await;
                        }
                    }
                    RunnerOutcome::Completed
                } else {
                    let mut adapter = ClaudeRunnerAdapter::new(prompt, task_id);
                    if let Some(sp) = &self.system_prompt {
                        adapter = adapter.with_system_prompt(sp.clone());
                    }
                    // If the stage row already carries a captured
                    // session id from a previous (interrupted) run,
                    // pass it through so the upstream wrapper resumes
                    // the same claude conversation rather than starting
                    // fresh. A0 — intra-stage session continuation
                    // per SCOPE.md hard rule #1.
                    if let Some(store) = self.store.as_ref() {
                        if let Ok(Some(stage)) = store.get_stage(stage_id).await {
                            if let Some(session_id) = stage.session_id {
                                if !session_id.is_empty() {
                                    adapter = adapter.with_resume_id(session_id);
                                }
                            }
                        }
                    }
                    adapter.run(sub_ctx).await
                };
                match outcome {
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

            // REVIEW blocking-gate evaluation. Runs only for stages
            // flagged review; reads the handover the inner adapter
            // wrote, parses the `PASS:` / `FAIL:` sentinel, and turns
            // the verdict into stage status. Missing or ambiguous
            // sentinel is treated as failure — a silent gate is not
            // permitted to wave a job through. See module docs and
            // `review_gate.rs` for the contract.
            if stage.is_review {
                match evaluate_review_gate(ctx.worktree_path.as_deref(), ctx.job_id, stage_id).await
                {
                    Ok(ReviewVerdict::Pass { reason }) => {
                        tracing::info!(stage = stage.title, %reason, "review gate passed");
                        // Step 4 shadow-mode: a PASS verdict may carry a
                        // single `ScopePatch` proposal in the same
                        // handover body. The runtime parses, persists to
                        // `DOCS/SCOPE-PROPOSED.md`, and emits a
                        // `ScopePatchProposed` envelope. Nothing merges —
                        // human approval lands in Step 6. Failures here
                        // are observable-but-non-fatal in shadow mode
                        // (per `scope_patch_emit::EmitOutcome` doc), so
                        // they map onto warn-level structured logs
                        // rather than flipping the gate's verdict; Step
                        // 5 promotes the parse errors to FAIL reasons.
                        if let Some(wt) = ctx.worktree_path.as_deref() {
                            let body =
                                tokio::fs::read_to_string(handover_path(wt, ctx.job_id, stage_id))
                                    .await
                                    .unwrap_or_default();
                            let review_id = ReviewId::new();
                            match emit_from_handover(
                                ctx.bus.as_ref(),
                                wt,
                                ctx.job_id,
                                stage_id,
                                review_id,
                                &body,
                            )
                            .await
                            {
                                EmitOutcome::Emitted(patch_id) => {
                                    tracing::info!(
                                        stage = stage.title,
                                        %patch_id,
                                        "scope-patch proposal recorded in shadow mode"
                                    );
                                }
                                EmitOutcome::NoBlock => {}
                                EmitOutcome::MultipleBlocks => {
                                    tracing::warn!(
                                        stage = stage.title,
                                        "review handover carried multiple SCOPE-PATCH blocks; \
                                         shadow mode ignores all of them (Step 5 will FAIL)"
                                    );
                                }
                                EmitOutcome::Malformed(reason) => {
                                    tracing::warn!(
                                        stage = stage.title,
                                        %reason,
                                        "review handover carried a malformed SCOPE-PATCH block; \
                                         shadow mode ignores it (Step 5 will FAIL)"
                                    );
                                }
                                EmitOutcome::SideEffectFailed(reason) => {
                                    tracing::warn!(
                                        stage = stage.title,
                                        %reason,
                                        "scope-patch proposal side-effect failed; continuing"
                                    );
                                }
                            }
                        }
                    }
                    Ok(ReviewVerdict::Fail { reason }) => {
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
                        tracing::warn!(
                            stage = stage.title,
                            %reason,
                            "review gate failed; aborting template run"
                        );
                        return RunnerOutcome::Failed {
                            reason: format!("review gate failed: {reason}"),
                        };
                    }
                    Err(err) => {
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
                        let reason = format!("review gate verdict unparseable: {err}");
                        tracing::warn!(stage = stage.title, %reason, "review gate aborted run");
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
            // The next iteration's REVIEW pre-check (if any) keys off
            // this stage's id to locate its handover. Updated only on
            // the Passed exit path; a Failed stage short-circuits via
            // `return` above and never reaches here.
            prev_stage_id = Some(stage_id);
        }
        RunnerOutcome::Completed
    }
}

/// Outcome of the REVIEW-stage diff-verify pre-check. `Skipped`
/// distinguishes "ran and nothing to verify" / "could not run because
/// the prior handover is absent" from `Pass` (verified at least one
/// path) so the structured log can be read with that distinction
/// intact. The caller treats `Pass` and `Skipped` identically — both
/// allow the inner adapter to run — but the log line is different.
#[derive(Debug)]
enum PreCheckOutcome {
    Pass,
    Skipped,
    Fail(String),
}

/// Run the Layer-1 diff-verify pre-check for a REVIEW stage. Reads
/// the prior stage's handover from `worktree`, lists the worktree's
/// changed paths via `codeless_adapters_host::changed_files`, and
/// asks `diff_verify::verify_handover` whether every `Done`-claimed
/// path is present.
///
/// The `git` invocation is wrapped in `tokio::task::spawn_blocking`
/// because `changed_files` shells out synchronously; running it on
/// the reactor would stall every other in-flight stage on a slow
/// repo. The blocking task's `JoinError` collapses into a Skipped
/// outcome — we would rather miss a verification than fail a stage on
/// a runtime bug in our own thread pool.
async fn run_diff_verify_precheck(
    worktree: &std::path::Path,
    job_id: codeless_types::JobId,
    prev_stage_id: StageId,
) -> PreCheckOutcome {
    let handover_file = handover_path(worktree, job_id, prev_stage_id);
    let body = match tokio::fs::read_to_string(&handover_file).await {
        Ok(b) => b,
        Err(err) => {
            tracing::info!(
                ?err,
                path = %handover_file.display(),
                "diff-verify pre-check: prior handover absent; skipping"
            );
            return PreCheckOutcome::Skipped;
        }
    };
    let handover = match codeless_types::Handover::from_markdown(&body) {
        Ok(h) => h,
        Err(err) => {
            tracing::warn!(
                ?err,
                path = %handover_file.display(),
                "diff-verify pre-check: prior handover unparseable; skipping (H7 should catch this on the next write)"
            );
            return PreCheckOutcome::Skipped;
        }
    };

    let wt = worktree.to_path_buf();
    // The base ref the worktree forked from: `main` is the default in
    // every codeless repo today (see `Repo::default_branch`). When the
    // ref does not resolve, `changed_files` falls back to listing
    // every commit on the current branch, which is the right answer
    // for a fresh worktree whose `main` was pruned.
    let diff_paths = match tokio::task::spawn_blocking(move || {
        codeless_adapters_host::changed_files(&wt, "main")
    })
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(err)) => {
            tracing::warn!(
                ?err,
                "diff-verify pre-check: git enumeration failed; skipping"
            );
            return PreCheckOutcome::Skipped;
        }
        Err(err) => {
            tracing::warn!(?err, "diff-verify pre-check: join error; skipping");
            return PreCheckOutcome::Skipped;
        }
    };

    match verify_handover(&handover, &diff_paths) {
        DiffVerifyOutcome::Pass { verified } => {
            tracing::info!(
                count = verified.len(),
                "diff-verify pre-check: every claimed path resolved to a diff entry"
            );
            PreCheckOutcome::Pass
        }
        DiffVerifyOutcome::NothingToVerify => {
            tracing::info!(
                path = %handover_file.display(),
                "diff-verify pre-check: prior handover `Done` named no path-shaped tokens; nothing to verify"
            );
            PreCheckOutcome::Skipped
        }
        DiffVerifyOutcome::Fail { missing } => {
            PreCheckOutcome::Fail(diff_verify_fail_reason(&missing))
        }
    }
}

/// Synthesise a minimal handover with a `PASS:` sentinel for the
/// mock-runner REVIEW path. Only used when `use_mock_runner` is on;
/// the real claude adapter writes its own handover from model output.
async fn write_mock_review_handover(
    worktree: &std::path::Path,
    job_id: codeless_types::JobId,
    stage_id: StageId,
) {
    let h = codeless_types::Handover {
        done: vec!["mock review stage".to_string()],
        next: vec!["next stage runs".to_string()],
        what_you_need_to_know: vec![
            "PASS: mock runner auto-passes REVIEW gates so the dev/demo \
             event shape stays consistent with the real adapter."
                .to_string(),
        ],
        open_questions: Vec::new(),
    };
    if let Err(err) = crate::handover::write_handover(worktree, job_id, stage_id, &h).await {
        tracing::warn!(?err, "mock review handover write failed; gate will fail");
    }
}

/// Read the handover the inner adapter wrote for this stage and
/// parse its `PASS:` / `FAIL:` sentinel into a verdict. The error
/// type is `String` because the failure path collapses three
/// underlying causes — no worktree on the context, the handover file
/// missing/unreadable, and the sentinel being missing or ambiguous —
/// into a single "the gate could not produce a verdict" outcome that
/// the caller treats as failure regardless of cause.
async fn evaluate_review_gate(
    worktree: Option<&std::path::Path>,
    job_id: codeless_types::JobId,
    stage_id: StageId,
) -> Result<ReviewVerdict, String> {
    let worktree = worktree.ok_or_else(|| {
        "review gate has no worktree on its runner context; cannot read handover".to_string()
    })?;
    let path = handover_path(worktree, job_id, stage_id);
    let body = tokio::fs::read_to_string(&path)
        .await
        .map_err(|err| format!("read handover {}: {err}", path.display()))?;
    parse_review_verdict(&body).map_err(|err: VerdictParseError| err.to_string())
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
    fn review_prompt_carries_sentinel_contract() {
        // The REVIEW prompt must instruct the model on the exact
        // `PASS:` / `FAIL:` sentinel the runtime parses after the
        // stage runs. Asserting on the sentinel tokens (rather than
        // just the word "REVIEW") protects the wire-level contract
        // between the prompt and `review_gate::parse_review_verdict`.
        let r = TemplateRunner::new(template_with_stages(&["REVIEW gate", "after"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[0], 2, None);
        assert!(prompt.contains("REVIEW stage"));
        assert!(prompt.contains("PASS:"));
        assert!(prompt.contains("FAIL:"));
        assert!(prompt.contains("blocking gate"));
    }

    #[tokio::test]
    async fn evaluate_review_gate_reads_pass_from_handover() {
        use codeless_types::{Handover, JobId};
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let h = Handover {
            done: vec!["did stuff".into()],
            next: vec!["next thing".into()],
            what_you_need_to_know: vec!["PASS: invariants hold".into()],
            open_questions: Vec::new(),
        };
        crate::handover::write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .unwrap();

        let verdict = evaluate_review_gate(Some(tmp.path()), job_id, stage_id)
            .await
            .expect("verdict parses");
        match verdict {
            ReviewVerdict::Pass { reason } => assert_eq!(reason, "invariants hold"),
            other => panic!("expected Pass, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn evaluate_review_gate_reads_fail_from_handover() {
        use codeless_types::{Handover, JobId};
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let h = Handover {
            done: vec!["FAIL: WORK touched CLAUDE.md".into()],
            next: vec!["fix it".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .unwrap();

        let verdict = evaluate_review_gate(Some(tmp.path()), job_id, stage_id)
            .await
            .expect("verdict parses");
        match verdict {
            ReviewVerdict::Fail { reason } => assert_eq!(reason, "WORK touched CLAUDE.md"),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn evaluate_review_gate_missing_sentinel_is_error() {
        use codeless_types::{Handover, JobId};
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let h = Handover {
            done: vec!["forgot the sentinel".into()],
            next: vec!["nothing".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .unwrap();

        let err = evaluate_review_gate(Some(tmp.path()), job_id, stage_id)
            .await
            .expect_err("missing sentinel is an error");
        assert!(err.contains("did not contain"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn evaluate_review_gate_missing_handover_is_error() {
        use codeless_types::JobId;
        let tmp = tempfile::tempdir().unwrap();
        let err = evaluate_review_gate(Some(tmp.path()), JobId::new(), StageId::new())
            .await
            .expect_err("missing handover is an error");
        assert!(err.contains("read handover"));
    }

    #[tokio::test]
    async fn evaluate_review_gate_no_worktree_is_error() {
        use codeless_types::JobId;
        let err = evaluate_review_gate(None, JobId::new(), StageId::new())
            .await
            .expect_err("no worktree is an error");
        assert!(err.contains("no worktree"));
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

    /// Build a real-on-disk git worktree seeded with one commit on a
    /// `feature` branch so `run_diff_verify_precheck` can shell out to
    /// `git` against the same paths the test asserts on. Returns the
    /// worktree root (the `TempDir` is held by the test to keep the
    /// directory alive).
    fn seed_worktree_with(paths: &[&str]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        for (op, args) in [
            ("init", vec!["init", "--initial-branch=main"]),
            ("config-name", vec!["config", "user.email", "t@e"]),
            ("config-email", vec!["config", "user.name", "t"]),
        ] {
            let out = std::process::Command::new("git")
                .current_dir(p)
                .args(&args)
                .output()
                .expect(op);
            assert!(out.status.success(), "git {op} failed");
        }
        std::fs::write(p.join("README.md"), "# seed\n").unwrap();
        let _ = std::process::Command::new("git")
            .current_dir(p)
            .args(["add", "."])
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .current_dir(p)
            .args(["commit", "-m", "seed"])
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .current_dir(p)
            .args(["checkout", "-b", "feature"])
            .output()
            .unwrap();
        for path in paths {
            let full = p.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, "x\n").unwrap();
        }
        let _ = std::process::Command::new("git")
            .current_dir(p)
            .args(["add", "."])
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .current_dir(p)
            .args(["commit", "-m", "stage work"])
            .output()
            .unwrap();
        tmp
    }

    #[tokio::test]
    async fn precheck_passes_when_handover_claims_match_diff() {
        use codeless_types::{Handover, JobId};
        let tmp = seed_worktree_with(&["crates/codeless-runtime/src/diff_verify.rs"]);
        let job_id = JobId::new();
        let prev = StageId::new();
        let h = Handover {
            done: vec!["added `crates/codeless-runtime/src/diff_verify.rs`".into()],
            next: vec!["review the diff".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, prev, &h)
            .await
            .unwrap();
        match run_diff_verify_precheck(tmp.path(), job_id, prev).await {
            PreCheckOutcome::Pass => {}
            other => panic!("expected Pass, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn precheck_fails_when_handover_claims_a_path_no_commit_touched() {
        use codeless_types::{Handover, JobId};
        let tmp = seed_worktree_with(&["a/b.rs"]);
        let job_id = JobId::new();
        let prev = StageId::new();
        let h = Handover {
            done: vec!["edited `unrelated/notes.md` and `a/b.rs`".into()],
            next: vec!["go".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, prev, &h)
            .await
            .unwrap();
        match run_diff_verify_precheck(tmp.path(), job_id, prev).await {
            PreCheckOutcome::Fail(reason) => {
                assert!(
                    reason.contains("unrelated/notes.md"),
                    "reason did not name the missing path: {reason}"
                );
                assert!(
                    !reason.contains("a/b.rs"),
                    "reason should not list verified paths as missing: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn precheck_skips_when_prior_handover_is_absent() {
        use codeless_types::JobId;
        let tmp = seed_worktree_with(&["a/b.rs"]);
        let job_id = JobId::new();
        // No write_handover call — the file deliberately does not
        // exist. The pre-check must not synthesise one; mock-runner
        // mode and "REVIEW as the first stage" both depend on this.
        match run_diff_verify_precheck(tmp.path(), job_id, StageId::new()).await {
            PreCheckOutcome::Skipped => {}
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn precheck_skips_when_done_names_no_paths() {
        use codeless_types::{Handover, JobId};
        let tmp = seed_worktree_with(&["a/b.rs"]);
        let job_id = JobId::new();
        let prev = StageId::new();
        let h = Handover {
            done: vec!["addressed R1 and bumped MSRV to 1.78".into()],
            next: vec!["next".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, prev, &h)
            .await
            .unwrap();
        match run_diff_verify_precheck(tmp.path(), job_id, prev).await {
            PreCheckOutcome::Skipped => {}
            other => panic!("expected Skipped, got {other:?}"),
        }
    }
}
