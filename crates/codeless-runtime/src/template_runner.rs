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
use codeless_types::review_gate::{PreCheckOutcome as WirePreCheck, ReviewVerdict as WireVerdict};
use codeless_types::{
    Event, FailureClass, ReviewId, StageId, StageStatus, StopReason, TaskId, TodoId, TodoKind,
};
use tokio_util::sync::CancellationToken;

use crate::auto_bypass_guard::ThrashingGuard;
use crate::claude_runner::ClaudeRunnerAdapter;
use crate::diff_verify::{
    fail_reason as diff_verify_fail_reason, verify_handover, DiffVerifyOutcome,
};
use crate::handover::handover_path;
use crate::review_gate::{parse_review_verdict_lenient, ReviewVerdict, VerdictParseError};
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::scope_patch_emit::{emit_from_handover, EmitOutcome};
use crate::state_machine::stage_trio_gate;
use crate::store::SqliteStore;
use crate::template::{JobTemplate, PlannedStage};
use crate::time::now_ms;
use crate::trio_emitter::{commit_stage_changes, find_trio_id, skip_pending_trio_rows};
use crate::verify_runner::{run_verify, HostVerifyExec, VerifyOutcome};

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
    /// Surface F thrashing guard. Shared across every `TemplateRunner`
    /// the factory builds so a single process's running jobs all
    /// agree on the consecutive-auto-bypass count, and rebuilt from
    /// the events table at the top of `run` so a driver restart does
    /// not reset the count. `None` (test-harness path) opts every
    /// stage failure into the historical halt behaviour — the guard
    /// has nothing to track, but `record_auto_bypass` would have no
    /// observer either.
    pub thrashing_guard: Option<Arc<ThrashingGuard>>,
    /// Operator-supplied comment to thread into the first stage's
    /// prompt under the same `# Operator comment` envelope the
    /// auto-bypass policy path uses. Originates in
    /// `ResumeJobArgs::next_stage_comment` for the Slack-driven
    /// `resume <job-id> "<comment>"` shape (and the equivalent UI
    /// affordance). Consumed exactly once: the runner seeds
    /// `next_stage_prefix` from it on the first iteration of `run`
    /// and clears the prefix after the next `stage_prompt` call so
    /// the guidance does not bleed into later, unrelated stages.
    pub pending_operator_comment: Option<String>,
    /// Path to the `codeless-mcp` binary. Threaded into each
    /// per-stage `ClaudeRunnerAdapter` so codeless-registered tools
    /// are available via MCP (stdio transport).
    pub mcp_binary_path: Option<String>,
}

impl TemplateRunner {
    pub fn new(template: JobTemplate) -> Self {
        Self {
            template,
            system_prompt: None,
            use_mock_runner: false,
            store: None,
            thrashing_guard: None,
            pending_operator_comment: None,
            mcp_binary_path: None,
        }
    }

    /// Seed the first stage's `# Operator comment` block from the
    /// caller-supplied free-text comment. Used by the runner factory
    /// to thread `ResumeJobArgs::next_stage_comment` into the next
    /// stage's prompt. An empty string is treated as `None` so a
    /// no-op payload does not produce a stray empty heading.
    pub fn with_pending_operator_comment(mut self, comment: Option<String>) -> Self {
        self.pending_operator_comment = comment.filter(|s| !s.is_empty());
        self
    }

    /// Attach a process-wide `ThrashingGuard` so this runner's
    /// auto-bypass decisions feed the same counter every other runner
    /// in the same server consults. The driver factory holds a single
    /// `Arc<ThrashingGuard>` and clones it into every
    /// `TemplateRunner::build`; the test harness leaves the field
    /// `None` and exercises the guard through its own module tests.
    pub fn with_thrashing_guard(mut self, guard: Arc<ThrashingGuard>) -> Self {
        self.thrashing_guard = Some(guard);
        self
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

    /// Set the `codeless-mcp` binary path for per-stage tool access.
    pub fn with_mcp_binary(mut self, path: impl Into<String>) -> Self {
        let s = path.into();
        self.mcp_binary_path = if s.is_empty() { None } else { Some(s) };
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
    /// Centralised post-classify handler: every `FailureAction::
    /// AutoBypass` branch consults the thrashing guard, halts when
    /// the guard says so (stamping `stop_reason` on the row first),
    /// and otherwise emits the `StageAutoBypassed` envelope plus
    /// records the bypass into the in-memory count. The five
    /// auto-bypass call sites in `run` differ only in the failure
    /// reason text and tracing line; the load-bearing thrash-or-emit
    /// shape lives here so a future tweak to the guard contract
    /// (e.g. a window-size change per Q1) lands in one place.
    async fn try_auto_bypass(
        &self,
        ctx: &RunnerContext,
        stage_id: StageId,
        task_id: TaskId,
        policy_name: &str,
        comment: &str,
        thrash_guard_applies: bool,
    ) -> AutoBypassDecision {
        if thrash_guard_applies {
            if let Some(guard) = self.thrashing_guard.as_ref() {
                if guard.would_breach(ctx.job_id) {
                    record_thrash_halt(self.store.as_deref(), ctx).await;
                    return AutoBypassDecision::Thrash;
                }
            }
        }
        emit_auto_bypass(
            ctx,
            stage_id,
            task_id,
            policy_name.to_string(),
            comment.to_string(),
        )
        .await;
        if thrash_guard_applies {
            if let Some(guard) = self.thrashing_guard.as_ref() {
                guard.record_auto_bypass(ctx.job_id);
            }
        }
        AutoBypassDecision::Advanced
    }

    fn stage_prompt(
        &self,
        planned: PlannedStage<'_>,
        total: usize,
        worktree: Option<&std::path::Path>,
        operator_comment: Option<&str>,
        prior_failure: Option<&str>,
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

        // Surface F auto-bypass thread-through: when the prior stage
        // failed under a job-level `AutoBypassPolicy`, the runner stamps
        // the policy's canned comment into the *next* stage's prompt
        // above everything else so the model reads the operator's
        // pre-authorised guidance before it sees the goal. Same envelope
        // shape `resume_job`'s `comment` argument uses (single `Operator
        // comment` heading) so the model parses one form, not two.
        let operator_block = match operator_comment {
            Some(text) if !text.is_empty() => format!("# Operator comment\n\n{text}\n\n"),
            _ => String::new(),
        };

        // Prior-attempt failure context: when the runner is re-
        // entering an ordinal whose latest attempt failed, the
        // failure summary (pre-check missing list + verdict reason +
        // stage-completed reason) is prepended above everything else
        // so the model reads what went wrong before it reads the
        // goal. Authored by `build_prior_failure_context`; absent on
        // fresh runs and on re-runs of bypassed stages.
        let prior_failure_block = match prior_failure {
            Some(text) if !text.is_empty() => format!("{text}\n\n"),
            _ => String::new(),
        };

        format!(
            "{prior_failure_block}{operator_block}{stage_docs_block}\
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

/// Build a markdown block summarising the previous failed attempt of
/// a stage so the next attempt's prompt carries the diagnosis. Reads
/// the same `events` table the wire log writes to (no separate
/// failure store) and collects, in order: review-pre-check `missing`
/// paths, review-verdict `reason`, stage-completed `reason` /
/// `status`. Returns `None` when no informative events exist for the
/// stage_id so a noise-only failure (e.g. crash before any event)
/// does not stamp an empty heading on the next prompt.
///
/// Failures here are warn-only at the call site: an unreadable
/// events table degrades to "no prior-failure block," not to a
/// runner abort. The next attempt then runs without the context,
/// matching pre-feature behaviour rather than blocking the resume.
pub(crate) async fn build_prior_failure_context(
    store: &SqliteStore,
    stage_id: StageId,
    ordinal: u32,
) -> sqlx::Result<Option<String>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT type, payload FROM events \
         WHERE stage_id = ? ORDER BY cursor",
    )
    .bind(stage_id.to_string())
    .fetch_all(store.pool())
    .await?;
    let mut pre_check_missing: Option<Vec<String>> = None;
    let mut verdict_reason: Option<String> = None;
    let mut stage_completed_reason: Option<String> = None;
    for row in rows {
        let ty: String = row.try_get("type")?;
        let payload: String = row.try_get("payload")?;
        let value: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match ty.as_str() {
            "review-pre-check" => {
                if let Some(outcome) = value.get("outcome") {
                    if outcome.get("outcome").and_then(|s| s.as_str()) == Some("fail") {
                        if let Some(missing) = outcome.get("missing").and_then(|m| m.as_array()) {
                            pre_check_missing = Some(
                                missing
                                    .iter()
                                    .filter_map(|v| v.as_str().map(str::to_owned))
                                    .collect(),
                            );
                        }
                    }
                }
            }
            "review-verdict" => {
                if let Some(verdict) = value.get("verdict") {
                    if verdict.get("verdict").and_then(|s| s.as_str()) != Some("pass") {
                        verdict_reason = verdict
                            .get("reason")
                            .and_then(|s| s.as_str())
                            .map(str::to_owned);
                    }
                }
            }
            "stage-completed" if value.get("status").and_then(|s| s.as_str()) == Some("failed") => {
                stage_completed_reason = value
                    .get("reason")
                    .and_then(|s| s.as_str())
                    .map(str::to_owned);
            }
            _ => {}
        }
    }
    if pre_check_missing.is_none() && verdict_reason.is_none() && stage_completed_reason.is_none() {
        return Ok(None);
    }
    let stage_num = ordinal + 1;
    let mut out = format!(
        "# Prior attempt failed (stage {stage_num})\n\n\
         This stage failed on the previous run. The failure context \
         below is provided so you can address the specific issue \
         rather than repeat it.\n"
    );
    if let Some(missing) = pre_check_missing {
        if !missing.is_empty() {
            out.push_str("\n## REVIEW pre-check: handover claimed paths not in diff\n\n");
            for path in missing {
                out.push_str("- ");
                out.push_str(&path);
                out.push('\n');
            }
        }
    }
    if let Some(reason) = verdict_reason {
        out.push_str("\n## REVIEW verdict\n\n");
        out.push_str(&reason);
        out.push('\n');
    }
    if let Some(reason) = stage_completed_reason {
        out.push_str("\n## Stage failure reason\n\n");
        out.push_str(&reason);
        out.push('\n');
    }
    Ok(Some(out))
}

#[async_trait]
impl Runner for TemplateRunner {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let planned = self.template.planned_stages();
        let total = planned.len();
        // Surface F thrashing guard: rebuild the in-memory count from
        // the persisted events table before the first stage runs. A
        // driver restart wipes the map but the wire log survives, so
        // a resumed job that was already at one auto-bypass picks up
        // the count where it left off and the next failure halts
        // instead of burning a fresh policy budget. Failures here are
        // warn-only: a missing rebuild degrades to "guard starts at
        // zero," not to a runner abort.
        if let (Some(guard), Some(store)) = (self.thrashing_guard.as_ref(), self.store.as_ref()) {
            if let Err(err) = guard.rebuild_from_store(store, ctx.job_id).await {
                tracing::warn!(
                    ?err,
                    "thrashing-guard: rebuild_from_store failed; starting at zero"
                );
            }
        }
        // Resume-skip-passed: fetch prior stage rows once. On a fresh
        // job this is empty; on a resume it carries the per-ordinal
        // status history. The runner skips any ordinal whose latest
        // attempt is `Passed`, reusing that attempt's stage_id as the
        // prev pointer so the next REVIEW's diff-verify pre-check
        // reads the correct prior handover. Without this, every
        // resume restarts at ordinal 0, re-running already-passed
        // stages as expensive no-ops.
        // Both maps are derived from the same `list_stages_for_job`
        // query so they stay consistent on which attempt of an
        // ordinal "won". An ordinal can be in at most one map: the
        // latest attempt wins, and `Passed`/bypassed beats `Failed`
        // even if a stale Failed row appears earlier in insertion
        // order. The Failed map drives the prior-failure-context
        // block injected into the re-attempt prompt (so the next run
        // sees what the previous run got wrong).
        let (prior_passed_by_ord, prior_failed_by_ord): (
            std::collections::HashMap<u32, StageId>,
            std::collections::HashMap<u32, StageId>,
        ) = match self.store.as_ref() {
            Some(store) => match store.list_stages_for_job(ctx.job_id).await {
                Ok(rows) => {
                    #[derive(Clone, Copy)]
                    enum LatestKind {
                        SkipEligible,
                        Failed,
                    }
                    let mut latest: std::collections::HashMap<u32, (StageId, LatestKind)> =
                        std::collections::HashMap::new();
                    for s in rows {
                        // Passed stages are skipped (success short-
                        // circuit). Failed-but-bypassed stages are
                        // ALSO skipped — the operator advanced past
                        // them via resume_job's `bypass` argument,
                        // so a re-run would either repeat the same
                        // failure or invent a different one. Bypass
                        // is the forward-advance signal; status stays
                        // Failed for audit.
                        let kind = if matches!(s.stage.status, StageStatus::Passed)
                            || s.stage.bypassed_at.is_some()
                        {
                            Some(LatestKind::SkipEligible)
                        } else if matches!(s.stage.status, StageStatus::Failed) {
                            Some(LatestKind::Failed)
                        } else {
                            None
                        };
                        if let Some(kind) = kind {
                            // Last write wins; list_stages_for_job
                            // returns rows in insertion order so the
                            // latest attempt overwrites any earlier
                            // entry for the same ordinal.
                            latest.insert(s.stage.ordinal, (s.stage.id, kind));
                        }
                    }
                    let mut passed = std::collections::HashMap::new();
                    let mut failed = std::collections::HashMap::new();
                    for (ord, (id, kind)) in latest {
                        match kind {
                            LatestKind::SkipEligible => {
                                passed.insert(ord, id);
                            }
                            LatestKind::Failed => {
                                failed.insert(ord, id);
                            }
                        }
                    }
                    (passed, failed)
                }
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        "resume-skip: list_stages_for_job failed; will re-run all stages"
                    );
                    (
                        std::collections::HashMap::new(),
                        std::collections::HashMap::new(),
                    )
                }
            },
            None => (
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
            ),
        };
        // Tracks the stage_id of the most recent stage that finished
        // (Passed) so a REVIEW stage's diff-verify pre-check can read
        // the prior WORK stage's handover by key rather than rely on
        // mtime-ranked discovery. Set after each stage's StageCompleted
        // emit; consumed at the top of the next REVIEW iteration.
        let mut prev_stage_id: Option<StageId> = None;
        // Surface F auto-bypass: when the previous stage failed under a
        // job-level `AutoBypassPolicy`, the runner stamps the policy's
        // canned comment here so the next iteration's `stage_prompt`
        // call prepends it as an `Operator comment` block above the
        // goal. Cleared after the next stage's prompt is built — the
        // guidance threads exactly one stage forward, not the rest of
        // the run.
        //
        // Seeded once on entry from `pending_operator_comment` so the
        // Slack-driven `resume <job-id> "<comment>"` path threads the
        // operator's note into the first stage `TemplateRunner`
        // executes after the resume. Same envelope as the auto-bypass
        // path so the model sees one `# Operator comment` heading
        // regardless of which surface filled it.
        let mut next_stage_prefix: Option<String> = self.pending_operator_comment.clone();
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
            // Skip ordinals whose latest attempt already passed on a
            // prior run. Carry the prior attempt's stage_id forward
            // as `prev_stage_id` so the next REVIEW gate's
            // diff-verify pre-check reads the right handover.
            if let Some(prior_id) = prior_passed_by_ord.get(&(stage.index as u32)) {
                tracing::info!(
                    stage = stage.title,
                    ordinal = stage.index,
                    prior_stage_id = %prior_id,
                    "template runner: ordinal already passed on a prior run; skipping",
                );
                prev_stage_id = Some(*prior_id);
                continue;
            }
            // Scoped pause hook — `Before` stage. Consult the
            // schedule before any per-stage state lands so a
            // BeforeStage point halts the job exactly between the
            // previous stage's git-todo close and this stage's
            // StageStarted emit. The stage frame index is 0-based in
            // `stage.index`; pause points speak 1-based ordinals
            // (matching the YAML the operator wrote).
            if let Some(store) = self.store.as_deref() {
                let outcome = crate::scoped_pause_hook::check_and_pause(
                    store,
                    ctx.bus.as_ref(),
                    ctx.job_id,
                    &crate::scoped_pause_hook::TransitionPoint::BeforeStage {
                        stage_ordinal: (stage.index as u32) + 1,
                    },
                    &ctx.cancel,
                )
                .await;
                if outcome == crate::scoped_pause_hook::HookOutcome::Paused {
                    tracing::info!(
                        stage = stage.title,
                        ordinal = stage.index,
                        "template runner: scoped pause point fired before stage; halting"
                    );
                    return RunnerOutcome::Failed {
                        reason: "scoped pause point".into(),
                    };
                }
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
            // Per-stage persona override (D1). Resolution against
            // `personas` is enforced at job-submit (rpc/jobs.rs), so
            // by the time the runner sees the id it has already been
            // validated. A `None` row at run time still degrades to
            // the job-level system prompt — either the row was
            // deleted between submit and dispatch, or the test
            // harness wired the runner without a store. Both stay
            // honest: the per-stage override just doesn't apply.
            let stage_persona_id = stage.persona.map(str::to_owned);
            let stage_persona = match (&stage_persona_id, self.store.as_ref()) {
                (Some(id), Some(store)) => store.get_persona(id).await.ok().flatten(),
                _ => None,
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
                    persona_id: stage_persona_id.clone(),
                },
            )
            .await;

            // Closing-trio injection. The runtime owns these three rows
            // so a misbehaving runner cannot silently skip the closing
            // steps; later in this stage the runtime emits
            // `TodoUpdated` / `TodoCompleted` from `verify_runner`, the
            // handover writer, and the per-stage commit step. Ordinals
            // sit at the top of the `u32` range so the trio sorts last
            // no matter how many runner-authored todos arrive before
            // stage end. `(task_id, ordinal)` is the store's idempotency
            // key, so a recorder replay on restart is a no-op.
            publish_trio(&ctx, stage_id, task_id).await;

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
                        // One-shot operator override: when the
                        // `override_pre_check_and_resume` RPC stamped
                        // the job row, atomically consume the flag
                        // and skip the diff-verify gate for exactly
                        // this re-entry. Publishing the `Skipped`
                        // wire variant gives the same audit trail
                        // shape the model-skipped-it paths use, so
                        // the timeline still records that the gate
                        // ran (and the override is visible in the
                        // event-log via the prior `JobResumed` plus
                        // the operator comment that the RPC requires).
                        let override_consumed = if let Some(store) = self.store.as_deref() {
                            match store.take_precheck_override_once(ctx.job_id).await {
                                Ok(v) => v,
                                Err(err) => {
                                    tracing::warn!(
                                        error = %err,
                                        "failed to take precheck override flag; pre-check will run"
                                    );
                                    false
                                }
                            }
                        } else {
                            false
                        };
                        if override_consumed {
                            publish(
                                &ctx,
                                stage_id,
                                task_id,
                                Event::ReviewPreCheck {
                                    stage_id,
                                    outcome: WirePreCheck::Skipped,
                                },
                            )
                            .await;
                            tracing::info!(
                                stage = stage.title,
                                "diff-verify pre-check skipped by operator override"
                            );
                        } else {
                            let outcome = run_diff_verify_precheck(wt, ctx.job_id, prev).await;
                            // Mirror the internal outcome onto the wire so
                            // Surface A can render the same shape the
                            // tracing line already records. Emitted before
                            // any control-flow branch so the event lands
                            // regardless of whether the stage proceeds or
                            // auto-fails.
                            let wire = match &outcome {
                                PreCheckOutcome::Pass { verified } => WirePreCheck::Pass {
                                    verified: verified.clone(),
                                },
                                PreCheckOutcome::Skipped => WirePreCheck::Skipped,
                                PreCheckOutcome::NothingToVerify => WirePreCheck::NothingToVerify,
                                PreCheckOutcome::Fail { missing, .. } => WirePreCheck::Fail {
                                    missing: missing.clone(),
                                },
                            };
                            publish(
                                &ctx,
                                stage_id,
                                task_id,
                                Event::ReviewPreCheck {
                                    stage_id,
                                    outcome: wire,
                                },
                            )
                            .await;
                            match outcome {
                                PreCheckOutcome::Pass { .. }
                                | PreCheckOutcome::Skipped
                                | PreCheckOutcome::NothingToVerify => {}
                                PreCheckOutcome::Fail { reason, .. } => {
                                    // Pre-check rejection short-circuits
                                    // before the model runs; the gate's
                                    // verdict on the wire is `AutoFail`
                                    // (no model verdict to report). Pairs
                                    // with the ReviewPreCheck::Fail event
                                    // already published above.
                                    publish(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        Event::ReviewVerdict {
                                            stage_id,
                                            verdict: WireVerdict::AutoFail {
                                                reason: reason.clone(),
                                            },
                                        },
                                    )
                                    .await;
                                    let failure_reason =
                                        format!("diff-verify pre-check failed: {reason}");
                                    publish(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        Event::StageCompleted {
                                            stage_id,
                                            status: StageStatus::Failed,
                                            failure_class: Some(FailureClass::PreCheckFailed),
                                            failure_detail: Some(truncate_failure_detail(
                                                &failure_reason,
                                            )),
                                        },
                                    )
                                    .await;
                                    tracing::warn!(
                                        stage = stage.title,
                                        %reason,
                                        "diff-verify pre-check failed; review stage auto-failed without invoking model"
                                    );
                                    match classify_stage_failure(self.store.as_deref(), &ctx).await
                                    {
                                        FailureAction::Halt => {
                                            // Stamp the wire-level stop
                                            // reason before returning so
                                            // the UI can distinguish a
                                            // structural pre-check
                                            // rejection from a runner
                                            // crash and surface the
                                            // `override_pre_check_and_resume`
                                            // affordance. Best-effort:
                                            // a stamp failure here still
                                            // halts the job — the
                                            // driver's fallback writes
                                            // `RunnerCrash` and the
                                            // operator falls back to
                                            // the cancel-and-rerun flow.
                                            if let Some(store) = self.store.as_deref() {
                                                if let Ok(Some(mut job)) =
                                                    store.get_job(ctx.job_id).await
                                                {
                                                    job.stop_reason =
                                                        Some(StopReason::ReviewPreCheck);
                                                    if let Err(err) = store.update_job(&job).await {
                                                        tracing::warn!(
                                                            error = %err,
                                                            "failed to stamp ReviewPreCheck stop_reason"
                                                        );
                                                    }
                                                }
                                            }
                                            return RunnerOutcome::Failed {
                                                reason: failure_reason,
                                            };
                                        }
                                        FailureAction::AutoBypass {
                                            policy_name,
                                            comment,
                                            thrash_guard_applies,
                                        } => {
                                            match self
                                                .try_auto_bypass(
                                                    &ctx,
                                                    stage_id,
                                                    task_id,
                                                    &policy_name,
                                                    &comment,
                                                    thrash_guard_applies,
                                                )
                                                .await
                                            {
                                                AutoBypassDecision::Thrash => {
                                                    tracing::warn!(
                                                        stage = stage.title,
                                                        policy = %policy_name,
                                                        %failure_reason,
                                                        "auto-bypass thrashing guard fired at pre-check failure; halting job"
                                                    );
                                                    return RunnerOutcome::Failed {
                                                        reason: format!(
                                                        "auto-bypass thrashing: {failure_reason}"
                                                    ),
                                                    };
                                                }
                                                AutoBypassDecision::Advanced => {
                                                    tracing::info!(
                                                        stage = stage.title,
                                                        policy = %policy_name,
                                                        %failure_reason,
                                                        "auto-bypass: advancing past pre-check failure"
                                                    );
                                                    next_stage_prefix = Some(comment);
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                }
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
                // Prior-failure context: when re-entering an ordinal
                // whose latest attempt failed (not bypassed), thread
                // the failure summary forward as a separate prompt
                // block above the operator comment. The map was
                // computed once at run-start from
                // `list_stages_for_job`; failures to read events
                // here degrade to "no block," not to a runner abort.
                let prior_failure = if let (Some(store), Some(prior_id)) = (
                    self.store.as_ref(),
                    prior_failed_by_ord.get(&(stage.index as u32)),
                ) {
                    match build_prior_failure_context(store, *prior_id, stage.index as u32).await {
                        Ok(block) => block,
                        Err(err) => {
                            tracing::warn!(
                                ?err,
                                prior_stage_id = %prior_id,
                                "prior-failure-context: query failed; re-attempt will run without context"
                            );
                            None
                        }
                    }
                } else {
                    None
                };
                let prompt = self.stage_prompt(
                    *stage,
                    total,
                    ctx.worktree_path.as_deref(),
                    next_stage_prefix.as_deref(),
                    prior_failure.as_deref(),
                );
                // The operator-comment prefix threads exactly one stage
                // forward (Surface F). Clear it now so a stage that
                // passes does not carry the prior policy comment into a
                // later, unrelated stage.
                next_stage_prefix = None;
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
                    // Per-stage persona instructions override the
                    // job-level system prompt for this stage only
                    // (D1, D5). Inheritance order: stage-level
                    // `persona.instructions` -> job-level
                    // `system_prompt` -> runner default. Empty
                    // `instructions` on a resolved persona row would
                    // unset the prompt for the stage, so guard.
                    let stage_sp = stage_persona
                        .as_ref()
                        .map(|p| p.instructions.as_str())
                        .filter(|s| !s.is_empty());
                    if let Some(sp) = stage_sp {
                        adapter = adapter.with_system_prompt(sp.to_owned());
                    } else if let Some(sp) = &self.system_prompt {
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
                    if let Some(ref mcp) = self.mcp_binary_path {
                        adapter = adapter.with_mcp_binary(mcp.clone());
                    }
                    // Thread the store through so the adapter can flip
                    // the runtime-injected `Docs` trio row around the
                    // handover write. Without this the stage-completion
                    // gate would block on a `Pending` `Docs` row even
                    // when the handover landed cleanly.
                    if let Some(store) = self.store.as_ref() {
                        adapter = adapter.with_store(Arc::clone(store));
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
                                failure_class: Some(classify_runner_failure_reason(&reason)),
                                failure_detail: Some(truncate_failure_detail(&reason)),
                            },
                        )
                        .await;
                        match classify_stage_failure(self.store.as_deref(), &ctx).await {
                            FailureAction::Halt => {
                                tracing::warn!(
                                    stage = stage.title,
                                    %reason,
                                    "stage failed; aborting template run"
                                );
                                return RunnerOutcome::Failed { reason };
                            }
                            FailureAction::AutoBypass {
                                policy_name,
                                comment,
                                thrash_guard_applies,
                            } => {
                                match self
                                    .try_auto_bypass(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        &policy_name,
                                        &comment,
                                        thrash_guard_applies,
                                    )
                                    .await
                                {
                                    AutoBypassDecision::Thrash => {
                                        tracing::warn!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass thrashing guard fired at stage failure; halting job"
                                        );
                                        return RunnerOutcome::Failed {
                                            reason: format!("auto-bypass thrashing: {reason}"),
                                        };
                                    }
                                    AutoBypassDecision::Advanced => {
                                        tracing::info!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass: advancing past stage failure"
                                        );
                                        next_stage_prefix = Some(comment);
                                        continue;
                                    }
                                }
                            }
                        }
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
                        // Surface A: model-driven PASS verdict. Emitted
                        // before patch validation so a later AutoFail
                        // (scope-patch malformed / rejected) lands as
                        // a second, distinct verdict event rather than
                        // replacing this one.
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::ReviewVerdict {
                                stage_id,
                                verdict: WireVerdict::Pass {
                                    reason: reason.clone(),
                                },
                            },
                        )
                        .await;
                        // Step 5: a PASS verdict may carry a single
                        // `ScopePatch` proposal in the same handover
                        // body. The runtime parses, validates, persists
                        // to `DOCS/SCOPE-PROPOSED.md`, and emits a
                        // `ScopePatchProposed` envelope. Nothing merges
                        // — human approval lands in Step 6. Step 5
                        // promotes the parse-time and shape-time
                        // failure modes (multiple blocks, malformed
                        // block, mutable-set / evidence violation) to
                        // REVIEW-gate FAIL reasons so a bad proposal
                        // cannot ride a PASS verdict past the gate.
                        // `SideEffectFailed` stays warn-only: an I/O
                        // wobble writing the proposals file must not
                        // fail a stage whose handover otherwise cleared.
                        if let Some(wt) = ctx.worktree_path.as_deref() {
                            let body =
                                tokio::fs::read_to_string(handover_path(wt, ctx.job_id, stage_id))
                                    .await
                                    .unwrap_or_default();
                            let review_id = ReviewId::new();
                            let changed_paths = enumerate_changed_paths(wt).await;
                            let outcome = emit_from_handover(
                                ctx.bus.as_ref(),
                                wt,
                                ctx.job_id,
                                stage_id,
                                review_id,
                                &body,
                                &changed_paths,
                            )
                            .await;
                            let reject_reason: Option<String> = match outcome {
                                EmitOutcome::Emitted(patch_id) => {
                                    tracing::info!(
                                        stage = stage.title,
                                        %patch_id,
                                        "scope-patch proposal recorded"
                                    );
                                    None
                                }
                                EmitOutcome::NoBlock => None,
                                EmitOutcome::MultipleBlocks => Some(
                                    "review handover carried more than one SCOPE-PATCH block; \
                                     one patch per REVIEW"
                                        .to_string(),
                                ),
                                EmitOutcome::Malformed(reason) => {
                                    Some(format!("scope-patch block malformed: {reason}"))
                                }
                                EmitOutcome::Rejected(reason) => {
                                    Some(format!("scope-patch rejected: {reason}"))
                                }
                                EmitOutcome::SideEffectFailed(reason) => {
                                    tracing::warn!(
                                        stage = stage.title,
                                        %reason,
                                        "scope-patch proposal side-effect failed; continuing"
                                    );
                                    None
                                }
                            };
                            if let Some(reason) = reject_reason {
                                // The model said PASS but the scope-
                                // patch validator overrode it; the
                                // wire verdict is AutoFail (the gate
                                // closed without a clean model verdict).
                                publish(
                                    &ctx,
                                    stage_id,
                                    task_id,
                                    Event::ReviewVerdict {
                                        stage_id,
                                        verdict: WireVerdict::AutoFail {
                                            reason: reason.clone(),
                                        },
                                    },
                                )
                                .await;
                                let failure_reason = format!("review gate failed: {reason}");
                                publish(
                                    &ctx,
                                    stage_id,
                                    task_id,
                                    Event::StageCompleted {
                                        stage_id,
                                        status: StageStatus::Failed,
                                        failure_class: Some(FailureClass::ReviewPatchInvalid),
                                        failure_detail: Some(truncate_failure_detail(
                                            &failure_reason,
                                        )),
                                    },
                                )
                                .await;
                                tracing::warn!(
                                    stage = stage.title,
                                    %reason,
                                    "review gate failed at patch parse/validation"
                                );
                                match classify_stage_failure(self.store.as_deref(), &ctx).await {
                                    FailureAction::Halt => {
                                        return RunnerOutcome::Failed {
                                            reason: failure_reason,
                                        };
                                    }
                                    FailureAction::AutoBypass {
                                        policy_name,
                                        comment,
                                        thrash_guard_applies,
                                    } => {
                                        match self
                                            .try_auto_bypass(
                                                &ctx,
                                                stage_id,
                                                task_id,
                                                &policy_name,
                                                &comment,
                                                thrash_guard_applies,
                                            )
                                            .await
                                        {
                                            AutoBypassDecision::Thrash => {
                                                tracing::warn!(
                                                    stage = stage.title,
                                                    policy = %policy_name,
                                                    %failure_reason,
                                                    "auto-bypass thrashing guard fired at review-gate patch rejection; halting job"
                                                );
                                                return RunnerOutcome::Failed {
                                                    reason: format!(
                                                        "auto-bypass thrashing: {failure_reason}"
                                                    ),
                                                };
                                            }
                                            AutoBypassDecision::Advanced => {
                                                tracing::info!(
                                                    stage = stage.title,
                                                    policy = %policy_name,
                                                    %failure_reason,
                                                    "auto-bypass: advancing past review-gate patch rejection"
                                                );
                                                next_stage_prefix = Some(comment);
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(ReviewVerdict::Fail { reason }) => {
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::ReviewVerdict {
                                stage_id,
                                verdict: WireVerdict::Fail {
                                    reason: reason.clone(),
                                },
                            },
                        )
                        .await;
                        let failure_reason = format!("review gate failed: {reason}");
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::StageCompleted {
                                stage_id,
                                status: StageStatus::Failed,
                                failure_class: Some(FailureClass::ReviewFail),
                                failure_detail: Some(truncate_failure_detail(&failure_reason)),
                            },
                        )
                        .await;
                        tracing::warn!(
                            stage = stage.title,
                            %reason,
                            "review gate failed; aborting template run"
                        );
                        match classify_stage_failure(self.store.as_deref(), &ctx).await {
                            FailureAction::Halt => {
                                return RunnerOutcome::Failed {
                                    reason: failure_reason,
                                };
                            }
                            FailureAction::AutoBypass {
                                policy_name,
                                comment,
                                thrash_guard_applies,
                            } => {
                                match self
                                    .try_auto_bypass(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        &policy_name,
                                        &comment,
                                        thrash_guard_applies,
                                    )
                                    .await
                                {
                                    AutoBypassDecision::Thrash => {
                                        tracing::warn!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            %failure_reason,
                                            "auto-bypass thrashing guard fired at review-gate fail; halting job"
                                        );
                                        return RunnerOutcome::Failed {
                                            reason: format!(
                                                "auto-bypass thrashing: {failure_reason}"
                                            ),
                                        };
                                    }
                                    AutoBypassDecision::Advanced => {
                                        tracing::info!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            %failure_reason,
                                            "auto-bypass: advancing past review-gate fail"
                                        );
                                        next_stage_prefix = Some(comment);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => {
                        let reason = format!("review gate verdict unparseable: {err}");
                        // The sentinel parser refused the handover —
                        // no model verdict to report, so the wire
                        // verdict is AutoFail with the parser's reason.
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::ReviewVerdict {
                                stage_id,
                                verdict: WireVerdict::AutoFail {
                                    reason: reason.clone(),
                                },
                            },
                        )
                        .await;
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::StageCompleted {
                                stage_id,
                                status: StageStatus::Failed,
                                failure_class: Some(FailureClass::ReviewUnparseable),
                                failure_detail: Some(truncate_failure_detail(&reason)),
                            },
                        )
                        .await;
                        tracing::warn!(stage = stage.title, %reason, "review gate aborted run");
                        match classify_stage_failure(self.store.as_deref(), &ctx).await {
                            FailureAction::Halt => {
                                return RunnerOutcome::Failed { reason };
                            }
                            FailureAction::AutoBypass {
                                policy_name,
                                comment,
                                thrash_guard_applies,
                            } => {
                                match self
                                    .try_auto_bypass(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        &policy_name,
                                        &comment,
                                        thrash_guard_applies,
                                    )
                                    .await
                                {
                                    AutoBypassDecision::Thrash => {
                                        tracing::warn!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass thrashing guard fired at review-gate parse failure; halting job"
                                        );
                                        return RunnerOutcome::Failed {
                                            reason: format!("auto-bypass thrashing: {reason}"),
                                        };
                                    }
                                    AutoBypassDecision::Advanced => {
                                        tracing::info!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass: advancing past review-gate parse failure"
                                        );
                                        next_stage_prefix = Some(comment);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Closing steps: drive the `Checks` and `Git` trio rails
            // so the gate below can resolve. PR #24 shipped the gate
            // but only the `Docs` rail was wired in production — see
            // DOCS/sessions/2026-05-18-trio-gate-wiring-fix.md for
            // the diagnosis. This block runs whether or not the
            // stage authored a `verify:` list (no-verify stages
            // `Skip` the rail); the defensive `skip_pending_trio_rows`
            // at the tail covers any rail a future non-claude runner
            // forgets to drive.
            if let Some(store) = self.store.as_ref() {
                if !stage.verify.is_empty() {
                    publish(&ctx, stage_id, task_id, Event::VerifyStarted { stage_id }).await;
                    let outcome = run_verify(
                        &ctx,
                        task_id,
                        stage_id,
                        stage.verify,
                        &HostVerifyExec,
                        Some(store.as_ref()),
                    )
                    .await;
                    match outcome {
                        VerifyOutcome::Passed => {
                            publish(&ctx, stage_id, task_id, Event::VerifyPassed { stage_id })
                                .await;
                        }
                        VerifyOutcome::Failed {
                            step_index,
                            exit_code,
                        } => {
                            publish(
                                &ctx,
                                stage_id,
                                task_id,
                                Event::VerifyFailed {
                                    stage_id,
                                    exit_code,
                                },
                            )
                            .await;
                            let reason =
                                format!("verify step {step_index} failed (exit {exit_code})");
                            publish(
                                &ctx,
                                stage_id,
                                task_id,
                                Event::StageCompleted {
                                    stage_id,
                                    status: StageStatus::Failed,
                                    failure_class: Some(classify_runner_failure_reason(&reason)),
                                    failure_detail: Some(truncate_failure_detail(&reason)),
                                },
                            )
                            .await;
                            return RunnerOutcome::Failed { reason };
                        }
                    }
                } else if let Some(todo_id) =
                    find_trio_id(store.as_ref(), task_id, TodoKind::Checks).await
                {
                    publish(
                        &ctx,
                        stage_id,
                        task_id,
                        Event::TodoCompleted {
                            todo_id,
                            status: codeless_types::TodoStatus::Skipped,
                            failure_detail: None,
                        },
                    )
                    .await;
                }

                // Per-stage commit, drives the `Git` trio rail.
                // `commit_stage_changes` shells out to
                // `git add -A && git commit`, which respects the
                // worktree's `.gitignore` so build artefacts
                // (`target/`, `.codeless/worktrees/`, etc.) stay out
                // of the commit. `ctx.worktree_path` is `None` only
                // on the test-harness path; production callers
                // always populate it through the driver.
                if let Some(wt) = ctx.worktree_path.as_deref() {
                    let subject = format!("stage {}: {}", stage.index, stage.title);
                    if let Err(err) =
                        commit_stage_changes(&ctx, store.as_ref(), task_id, stage_id, wt, &subject)
                            .await
                    {
                        tracing::warn!(?err, %stage_id, "stage commit failed; gate will rely on Failed trio row");
                    }
                }

                // Defensive: any rail a runner didn't drive (mock,
                // future codex/anthropic) gets Skipped here so the
                // gate below can resolve instead of polling forever.
                skip_pending_trio_rows(&ctx, store.as_ref(), task_id, stage_id).await;
            }

            // Stage-completion gate. The trio rows injected at stage
            // entry must all be `Done` or `Skipped` before the runtime
            // is allowed to publish `StageCompleted{ Passed }`. Failed
            // stages bypass this gate above via `return RunnerOutcome::
            // Failed { .. }` — a failed run cares about surfacing the
            // failure, not about trio bookkeeping. The test path
            // (`self.store.is_none()`) cannot persist trio rows and so
            // degrades to passthrough; production paths (`with_store`)
            // hold here until the runtime emitters in `verify_runner`,
            // the handover writer, and the per-stage commit step flip
            // each row.
            if let Some(store) = self.store.as_ref() {
                match wait_for_trio_resolved(store, &ctx, stage_id, task_id).await {
                    TrioGateWaitOutcome::Resolved => {}
                    TrioGateWaitOutcome::Cancelled => {
                        return RunnerOutcome::Failed {
                            reason: "cancelled while waiting for stage trio resolution".into(),
                        };
                    }
                    TrioGateWaitOutcome::Failed { reason }
                    | TrioGateWaitOutcome::TimedOut { reason } => {
                        // Closing-trio gate surfaced a terminal
                        // failure (one of the trio rails ended
                        // `Failed`, or the gate ran past its max
                        // wait). Route through the same auto-bypass
                        // path as any other stage failure so a job
                        // configured with an `auto_bypass_policy`
                        // advances instead of stranding the operator
                        // with an "apparently running" stage that
                        // will never close on its own.
                        publish(
                            &ctx,
                            stage_id,
                            task_id,
                            Event::StageCompleted {
                                stage_id,
                                status: StageStatus::Failed,
                                failure_class: Some(classify_runner_failure_reason(&reason)),
                                failure_detail: Some(truncate_failure_detail(&reason)),
                            },
                        )
                        .await;
                        match classify_stage_failure(self.store.as_deref(), &ctx).await {
                            FailureAction::Halt => {
                                tracing::warn!(
                                    stage = stage.title,
                                    %reason,
                                    "trio gate failed; aborting template run"
                                );
                                return RunnerOutcome::Failed { reason };
                            }
                            FailureAction::AutoBypass {
                                policy_name,
                                comment,
                                thrash_guard_applies,
                            } => {
                                match self
                                    .try_auto_bypass(
                                        &ctx,
                                        stage_id,
                                        task_id,
                                        &policy_name,
                                        &comment,
                                        thrash_guard_applies,
                                    )
                                    .await
                                {
                                    AutoBypassDecision::Thrash => {
                                        tracing::warn!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass thrashing guard fired at trio gate failure; halting job"
                                        );
                                        return RunnerOutcome::Failed {
                                            reason: format!("auto-bypass thrashing: {reason}"),
                                        };
                                    }
                                    AutoBypassDecision::Advanced => {
                                        tracing::info!(
                                            stage = stage.title,
                                            policy = %policy_name,
                                            failure_reason = %reason,
                                            "auto-bypass: advancing past trio gate failure"
                                        );
                                        next_stage_prefix = Some(comment);
                                        continue;
                                    }
                                }
                            }
                        }
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
                    failure_class: None,
                    failure_detail: None,
                },
            )
            .await;
            // Surface F: a Passed stage is the doc's reset criterion
            // for the consecutive-auto-bypass count. Drop the count
            // back to zero so the next failure under the policy is
            // allowed one auto-bypass before the two-strikes guard
            // fires again (DOCS/AUTO-BYPASS-DECISIONS.md Q1).
            if let Some(guard) = self.thrashing_guard.as_ref() {
                guard.record_pass(ctx.job_id);
            }
            // The next iteration's REVIEW pre-check (if any) keys off
            // this stage's id to locate its handover. Updated only on
            // the Passed exit path; a Failed stage short-circuits via
            // `return` above and never reaches here.
            prev_stage_id = Some(stage_id);

            // Scoped pause hook — `After` stage. Fires once the
            // closing trio has resolved and StageCompleted is on the
            // wire, but before the loop selects the next stage. A
            // BeforeStage point on stage N+1 and an AfterStage point
            // on stage N are intentionally distinct triggers — the
            // operator may want to inspect the worktree at the end
            // of stage N rather than the start of stage N+1 (the
            // diff that just landed vs the one that is about to
            // land); the parser accepts both and the runtime fires
            // both.
            if let Some(store) = self.store.as_deref() {
                let outcome = crate::scoped_pause_hook::check_and_pause(
                    store,
                    ctx.bus.as_ref(),
                    ctx.job_id,
                    &crate::scoped_pause_hook::TransitionPoint::AfterStage {
                        stage_ordinal: (stage.index as u32) + 1,
                    },
                    &ctx.cancel,
                )
                .await;
                if outcome == crate::scoped_pause_hook::HookOutcome::Paused {
                    tracing::info!(
                        stage = stage.title,
                        ordinal = stage.index,
                        "template runner: scoped pause point fired after stage; halting"
                    );
                    return RunnerOutcome::Failed {
                        reason: "scoped pause point".into(),
                    };
                }
            }
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
/// Internal pre-check outcome. Mirrors the wire
/// `codeless_types::review_gate::PreCheckOutcome` shape so the caller
/// can publish the wire event without re-deriving the variant; the
/// `Fail` variant additionally carries the human-readable `reason`
/// the runner returns to `RunnerOutcome::Failed` so a single match
/// drives both the event emit and the control flow.
#[derive(Debug)]
enum PreCheckOutcome {
    Pass {
        verified: Vec<String>,
    },
    Skipped,
    NothingToVerify,
    Fail {
        reason: String,
        missing: Vec<String>,
    },
}

/// Enumerate the worktree's changed-file set against its base ref.
/// Shared between the REVIEW-stage diff-verify pre-check and the
/// Step 5 `Loosen`-patch evidence verifier — both need the same view
/// of "what did this worktree actually touch since branching off main".
///
/// Errors collapse into an empty vec with a warn-level log: the
/// callers treat "no diff information" as "cannot verify", which is
/// the right default (the patch validator returns the evidence-not-
/// in-diff Rejection rather than emitting a proposal it cannot back).
async fn enumerate_changed_paths(worktree: &std::path::Path) -> Vec<String> {
    let wt = worktree.to_path_buf();
    match tokio::task::spawn_blocking(move || codeless_adapters_host::changed_files(&wt, "main"))
        .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(err)) => {
            tracing::warn!(?err, "changed-file enumeration failed; returning empty set");
            Vec::new()
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                "changed-file enumeration join error; returning empty set"
            );
            Vec::new()
        }
    }
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
            PreCheckOutcome::Pass { verified }
        }
        DiffVerifyOutcome::NothingToVerify => {
            tracing::info!(
                path = %handover_file.display(),
                "diff-verify pre-check: prior handover `Done` named no path-shaped tokens; nothing to verify"
            );
            PreCheckOutcome::NothingToVerify
        }
        DiffVerifyOutcome::Fail { missing } => {
            let reason = diff_verify_fail_reason(&missing);
            // Wire variant carries the claimed-path strings only; the
            // candidate suggestions stay in the `reason` text that
            // RunnerOutcome::Failed surfaces, same as the existing
            // tracing line.
            let missing_paths: Vec<String> = missing.iter().map(|m| m.claimed.clone()).collect();
            PreCheckOutcome::Fail {
                reason,
                missing: missing_paths,
            }
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
        raw_tail: None,
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
    // Lenient parse: a missing sentinel on a substantive handover
    // (real Done + Next) implicit-PASSes. A model can still
    // explicitly FAIL, and a genuinely empty handover still halts.
    parse_review_verdict_lenient(&body)
        .map(|(verdict, lenient_reason)| {
            if let Some(reason) = lenient_reason {
                tracing::warn!(
                    %job_id,
                    %stage_id,
                    %reason,
                    "review gate accepted handover via lenient parser; agent forgot the sentinel grammar",
                );
            }
            verdict
        })
        .map_err(|err: VerdictParseError| err.to_string())
}

/// Decision the stage-failed handler makes when a stage hits a
/// `RunnerOutcome::Failed` outcome (Surface F). `Halt` preserves the
/// pre-Surface-F behaviour: emit `JobFailed` upstream and stop. The
/// runner halts on `Halt` for three reasons it must never bypass —
/// (a) the cap watcher cancelled the job (CostCap / WallClock),
/// (b) an external `stop_job` / `pause_job` cancelled the job, or
/// (c) the job has no `auto_bypass_policy` set. `AutoBypass` thread-
/// through advances to the next stage and threads the policy's canned
/// comment forward as an `Operator comment` block.
#[derive(Debug)]
enum FailureAction {
    Halt,
    AutoBypass {
        policy_name: String,
        comment: String,
        /// `false` for the `Relentless` policy variant
        /// (`AUTO-BYPASS-DECISIONS.md` Q7) — the runner skips
        /// `ThrashingGuard::would_breach` and never records into the
        /// per-job counter. Every other policy leaves this `true` and
        /// goes through the Q1 two-strikes window unchanged.
        thrash_guard_applies: bool,
    },
}

/// Surface F thrashing-guard outcome at each auto-bypass call site.
/// `Advanced` means the runner emitted `StageAutoBypassed` and the
/// caller should set `next_stage_prefix` and `continue`. `Thrash`
/// means the guard fired: `stop_reason = AutoBypassThrashing` is
/// already on the job row and the caller must return
/// `RunnerOutcome::Failed` with a thrash-tagged reason so the driver
/// transitions the job terminal.
#[derive(Debug)]
enum AutoBypassDecision {
    Advanced,
    Thrash,
}

/// Classify a `RunnerOutcome::Failed` outcome against the job row.
///
/// Cancellation (cap watcher OR external stop/pause) always wins over
/// auto-bypass — the operator's `stop_job` must not be silently
/// reinterpreted as "retry forever," and the cap-breach contract in
/// `DOCS/AUTO-BYPASS-DECISIONS.md` Q1 explicitly fences caps off from
/// the policy. A store-less runner (test harness) cannot read the
/// policy column, so it falls through to `Halt` — auto-bypass is a
/// production feature, not a unit-test default.
async fn classify_stage_failure(store: Option<&SqliteStore>, ctx: &RunnerContext) -> FailureAction {
    if ctx.cancel.is_cancelled() {
        return FailureAction::Halt;
    }
    let Some(store) = store else {
        return FailureAction::Halt;
    };
    let job = match store.get_job(ctx.job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => return FailureAction::Halt,
        Err(err) => {
            tracing::warn!(?err, "auto-bypass: get_job failed; halting as today");
            return FailureAction::Halt;
        }
    };
    // Any `stop_reason` already on the row means a terminal decision
    // landed before us — cap breach, runner crash, or external stop.
    // None of those are eligible for auto-bypass.
    if job.stop_reason.is_some() {
        return FailureAction::Halt;
    }
    match job.auto_bypass_policy {
        Some(policy) => {
            let policy_name = policy.policy_name().to_string();
            let comment = crate::auto_bypass_policy::policy_comment(&policy).to_string();
            let thrash_guard_applies = policy.thrash_guard_applies();
            FailureAction::AutoBypass {
                policy_name,
                comment,
                thrash_guard_applies,
            }
        }
        None => FailureAction::Halt,
    }
}

/// Publish the Surface F `StageAutoBypassed` envelope. Stamped with
/// the current wall clock so the recorder can persist `bypassed_at`
/// against the same instant the wire event carries — keeping the row
/// and the audit trail mutually consistent without a second time
/// lookup downstream.
/// Write `stop_reason = AutoBypassThrashing` onto the job row when
/// the guard fires. `RunnerOutcome::Failed` returning to the driver
/// would otherwise leave `stop_reason` `None` (the driver translates
/// the outcome to `JobStatus::Failed` without setting a reason), and
/// the UI's gate panel reads the reason to label the halt as policy
/// thrashing rather than a generic crash. Failures here are warn-only
/// — the wire-event audit trail still records the
/// `StageAutoBypassed` envelopes that led to the thrash, so a missing
/// `stop_reason` degrades to "thrash recorded but unlabelled" rather
/// than a lost halt.
async fn record_thrash_halt(store: Option<&SqliteStore>, ctx: &RunnerContext) {
    let Some(store) = store else {
        return;
    };
    let mut job = match store.get_job(ctx.job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(
                ?err,
                "thrashing-guard: get_job failed; halt missing stop_reason"
            );
            return;
        }
    };
    job.stop_reason = Some(codeless_types::StopReason::AutoBypassThrashing);
    if let Err(err) = store.update_job(&job).await {
        tracing::warn!(
            ?err,
            "thrashing-guard: update_job failed; halt missing stop_reason"
        );
    }
}

async fn emit_auto_bypass(
    ctx: &RunnerContext,
    stage_id: StageId,
    task_id: TaskId,
    policy_name: String,
    comment_used: String,
) {
    let applied_at = now_ms();
    publish(
        ctx,
        stage_id,
        task_id,
        Event::StageAutoBypassed {
            stage_id,
            policy_name,
            comment_used,
            applied_at,
        },
    )
    .await;
}

/// Map a stage's terminal failure-`reason` string onto a
/// `FailureClass`. SQLite infrastructure errors (`SQLITE_FULL`,
/// `SQLITE_IOERR`, `SQLITE_CORRUPT`, `SQLITE_CANTOPEN`,
/// `SQLITE_READONLY` — see `DOCS/AUTO-BYPASS-DECISIONS.md` Q1) must
/// route to `InfrastructureError` so the policy halts instead of
/// auto-bypassing a host-side failure that retrying the same SQL on
/// the same disk cannot survive. The signal we have at the
/// `RunnerError` emit sites is the formatted `reason: String`; sqlx's
/// `SqliteError` Display surfaces the extended result code as
/// `(code: NNN) message`, and the primary code is the low byte of
/// the extended code. Everything else stays `RunnerError`.
pub(crate) fn classify_runner_failure_reason(reason: &str) -> FailureClass {
    if let Some(code) = extract_sqlite_extended_code(reason) {
        return classify_sqlite_extended_code(code);
    }
    FailureClass::RunnerError
}

/// Pull the integer that follows the literal `(code: ` prefix that
/// sqlx-sqlite's `SqliteError` Display emits. Returns `None` for any
/// reason string that does not carry the marker, which is the
/// overwhelmingly common case — the helper is intentionally narrow so
/// a future runner panic that happens to contain the substring in
/// prose does not accidentally re-classify as infra.
fn extract_sqlite_extended_code(reason: &str) -> Option<i32> {
    const MARKER: &str = "(code: ";
    let start = reason.find(MARKER)?;
    let tail = &reason[start + MARKER.len()..];
    let end = tail.find(')')?;
    tail[..end].trim().parse::<i32>().ok()
}

/// Decide on the **primary** SQLite result code (low byte of the
/// extended code). The infra set is locked in
/// `DOCS/AUTO-BYPASS-DECISIONS.md` Q1; expanding it is a SCOPE-level
/// change, not a one-line tweak here.
fn classify_sqlite_extended_code(extended: i32) -> FailureClass {
    match extended & 0xff {
        // SQLITE_READONLY (8), SQLITE_IOERR (10), SQLITE_CORRUPT (11),
        // SQLITE_FULL (13), SQLITE_CANTOPEN (14). Extended siblings
        // (e.g. SQLITE_IOERR_FSTAT = 1546, primary 10) collapse onto
        // the primary code by the mask above and are intentionally
        // covered.
        8 | 10 | 11 | 13 | 14 => FailureClass::InfrastructureError,
        _ => FailureClass::RunnerError,
    }
}

/// Short summary string for `failure_detail`. Trims to ~200 chars on
/// a char boundary so the SQLite column stays cheap to render and the
/// SSE envelope is small; the full transcript is on disk and the UI
/// links to it from the badge.
fn truncate_failure_detail(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX - 1).collect();
        out.push('…');
        out
    }
}

/// Emit the three closing-trio `TodoAdded` envelopes for a stage's
/// terminal task. Ordinals occupy the top of the `u32` range so the
/// trio always sorts to the bottom of the per-task checklist, no
/// matter how many runner-authored todos appear between stage entry
/// and stage end. Title strings are short by construction — the UI
/// row is one line and `WORKFLOW.md` caps the emit-site title around
/// 200 chars.
async fn publish_trio(ctx: &RunnerContext, stage_id: StageId, task_id: TaskId) {
    const TRIO_ORDINAL_BASE: u32 = u32::MAX - 2;
    for (offset, kind) in TodoKind::TRIO.iter().enumerate() {
        let title = match kind {
            TodoKind::Checks => "checks (cargo test / clippy / fmt)",
            TodoKind::Docs => "docs (handover + session log)",
            TodoKind::Git => "git (commit + push)",
            _ => continue,
        };
        publish(
            ctx,
            stage_id,
            task_id,
            Event::TodoAdded {
                todo_id: TodoId::new(),
                task_id,
                ordinal: TRIO_ORDINAL_BASE + offset as u32,
                title: title.into(),
                kind: *kind,
            },
        )
        .await;
    }
}

/// Poll-loop on the trio gate until `stage_trio_gate` returns true or
/// the context is cancelled. The runtime refuses to emit
/// `StageCompleted{ status: Passed }` until the trio is resolved on
/// the stage's terminal task; a misbehaving runner that short-circuits
/// one of the closing steps is held here rather than waved through.
/// A sqlx error is treated as a transient block — log, sleep, retry —
/// because dropping a stage's terminal envelope on a flaky DB read
/// would corrupt the recorder's view of the run.
///
/// Returns `false` only when the context was cancelled before the gate
/// resolved; the caller is expected to surface that as a failure
/// rather than emit a successful `StageCompleted`.
/// Outcome of the wait-on-closing-trio loop.
///
/// `Resolved` lets the stage pass; everything else routes the stage
/// through the auto-bypass-eligible failure path. The previous wait
/// loop returned `bool` and silently spun forever on a failed trio
/// row (the bug that stranded job `01KRX4ZPF...` for 90 minutes),
/// so a dedicated enum is now the only way to keep the gate honest:
/// callers must handle every variant explicitly.
enum TrioGateWaitOutcome {
    Resolved,
    /// At least one trio row landed `Failed`. The `reason` is a
    /// human-readable summary built from the per-rail failure_detail
    /// (`"docs: write handover: Permission denied"` etc.) for the
    /// stage's `failure_detail` and the auto-bypass card.
    Failed {
        reason: String,
    },
    /// The gate spun for `wait_max` without resolving. Treated as a
    /// stage failure under `FailureClass::RunnerError`. A timeout
    /// here means a trio resolver is wedged — the failure surfaces
    /// the wait duration and which rails were still pending so the
    /// operator has somewhere to start.
    TimedOut {
        reason: String,
    },
    /// Cancellation token fired. Caller emits the existing
    /// "cancelled while waiting" failure; no auto-bypass routing.
    Cancelled,
}

/// Maximum wall time the closing-trio gate may wait for the three
/// rails to resolve. In healthy runs every resolver (claude handover
/// write, verify_runner step, `git commit`) finishes in single-digit
/// seconds; five minutes is a generous safety margin that still
/// guarantees the operator sees a failure rather than an apparently
/// "running" stage if a resolver wedges. Tuned per the analysis of
/// job `01KRX4ZPF...` (the docs rail failed in ~400ms but the gate
/// then polled silently for 88 minutes before the operator stopped
/// it manually).
const TRIO_GATE_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// Cadence at which the runner emits `StageTrioGateWaiting` while
/// the gate is `Pending`. Short enough that an operator who opens
/// the job page mid-wait sees the heartbeat within a few seconds;
/// long enough not to flood the event stream during long verify
/// commands.
const TRIO_GATE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);

async fn wait_for_trio_resolved(
    store: &SqliteStore,
    ctx: &RunnerContext,
    stage_id: StageId,
    task_id: TaskId,
) -> TrioGateWaitOutcome {
    const POLL: std::time::Duration = std::time::Duration::from_millis(100);
    let started = std::time::Instant::now();
    let mut last_heartbeat = std::time::Instant::now()
        .checked_sub(TRIO_GATE_HEARTBEAT)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        if ctx.cancel.is_cancelled() {
            tracing::warn!(?stage_id, %task_id, "stage trio gate cancelled before resolution");
            return TrioGateWaitOutcome::Cancelled;
        }
        match stage_trio_gate(store, task_id).await {
            Ok(crate::store::TrioGateOutcome::Resolved) => {
                return TrioGateWaitOutcome::Resolved;
            }
            Ok(crate::store::TrioGateOutcome::Failed { failures }) => {
                let reason = format_trio_failures(&failures);
                tracing::warn!(?stage_id, %task_id, %reason, "stage trio gate: failed rails");
                return TrioGateWaitOutcome::Failed { reason };
            }
            Ok(crate::store::TrioGateOutcome::Pending) => {
                if started.elapsed() >= TRIO_GATE_MAX_WAIT {
                    let waiting_on = trio_pending_kinds(store, task_id).await;
                    let pending = format_pending_kinds(&waiting_on);
                    let reason = format!(
                        "trio gate timed out after {}s waiting on {pending}",
                        TRIO_GATE_MAX_WAIT.as_secs()
                    );
                    tracing::error!(?stage_id, %task_id, %reason, "stage trio gate: timed out");
                    return TrioGateWaitOutcome::TimedOut { reason };
                }
                if last_heartbeat.elapsed() >= TRIO_GATE_HEARTBEAT {
                    let waiting_on = trio_pending_kinds(store, task_id).await;
                    let elapsed_ms =
                        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                    publish(
                        ctx,
                        stage_id,
                        task_id,
                        Event::StageTrioGateWaiting {
                            stage_id,
                            waiting_on,
                            elapsed_ms,
                        },
                    )
                    .await;
                    last_heartbeat = std::time::Instant::now();
                }
            }
            Err(err) => {
                tracing::warn!(?err, ?stage_id, "stage trio gate query failed; retrying");
            }
        }
        tokio::select! {
            _ = ctx.cancel.cancelled() => {}
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

/// List the trio rails still in a non-terminal status. Used by the
/// heartbeat event and the timeout reason. Returns the rails in
/// `TodoKind::TRIO` order so UI sort is deterministic. A store-read
/// error is logged and returns an empty slice; the heartbeat
/// degrades to "waiting on (unknown)" rather than tearing the loop
/// down.
async fn trio_pending_kinds(store: &SqliteStore, task_id: TaskId) -> Vec<codeless_types::TodoKind> {
    let todos = match store.list_todos_for_task(task_id).await {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!(?err, %task_id, "trio_pending_kinds: list_todos failed");
            return Vec::new();
        }
    };
    codeless_types::TodoKind::TRIO
        .iter()
        .copied()
        .filter(|kind| {
            todos
                .iter()
                .find(|t| t.kind == *kind)
                .map(|t| {
                    matches!(
                        t.status,
                        codeless_types::TodoStatus::Pending
                            | codeless_types::TodoStatus::InProgress
                    )
                })
                .unwrap_or(true)
        })
        .collect()
}

fn format_trio_failures(failures: &[crate::store::TrioFailure]) -> String {
    failures
        .iter()
        .map(|f| match f.reason.as_deref() {
            Some(reason) => format!("{} rail failed: {reason}", kind_label(f.kind)),
            None => format!("{} rail failed", kind_label(f.kind)),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_pending_kinds(kinds: &[codeless_types::TodoKind]) -> String {
    if kinds.is_empty() {
        return "(unknown)".to_string();
    }
    kinds
        .iter()
        .map(|k| kind_label(*k))
        .collect::<Vec<_>>()
        .join(", ")
}

fn kind_label(kind: codeless_types::TodoKind) -> &'static str {
    match kind {
        codeless_types::TodoKind::Checks => "checks",
        codeless_types::TodoKind::Docs => "docs",
        codeless_types::TodoKind::Git => "git",
        codeless_types::TodoKind::Runner => "runner",
        codeless_types::TodoKind::Planner => "planner",
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

    /// Pin every locked-in infra primary code from
    /// `DOCS/AUTO-BYPASS-DECISIONS.md` Q1 to `InfrastructureError`,
    /// every exclusion-list code to `RunnerError`, and a couple of
    /// real-world extended-code observations to confirm the
    /// primary-code mask is applied (and not bypassed by a future
    /// "match the exact integer" refactor).
    #[test]
    fn classify_runner_failure_reason_maps_sqlite_codes() {
        // Primary-code infra set: SQLITE_READONLY, SQLITE_IOERR,
        // SQLITE_CORRUPT, SQLITE_FULL, SQLITE_CANTOPEN.
        for code in [8, 10, 11, 13, 14] {
            let reason = format!("(code: {code}) host disk failure");
            assert_eq!(
                classify_runner_failure_reason(&reason),
                FailureClass::InfrastructureError,
                "primary code {code} must classify as InfrastructureError",
            );
        }

        // Extended siblings collapse onto the same primary code:
        //   1546 = SQLITE_IOERR_FSTAT     -> primary 10
        //   2826 = SQLITE_IOERR_READ      -> primary 10
        //   1034 = SQLITE_READONLY_RECOVERY -> primary 8
        //    779 = SQLITE_CORRUPT_VTAB    -> primary 11
        //   3850 = SQLITE_CANTOPEN_DIRTYWAL -> primary 14
        for code in [1546, 2826, 1034, 779, 3850] {
            let reason = format!("(code: {code}) extended-code surface");
            assert_eq!(
                classify_runner_failure_reason(&reason),
                FailureClass::InfrastructureError,
                "extended code {code} must classify by its primary byte",
            );
        }

        // Explicit exclusion list — runner-side bugs that auto-bypass
        // is correct to retry against, plus SQLITE_NOTADB which the
        // SCOPE decision keeps in RunnerError on purpose.
        for code in [1, 5, 6, 19, 20, 21, 25, 26] {
            let reason = format!("(code: {code}) runner-side fault");
            assert_eq!(
                classify_runner_failure_reason(&reason),
                FailureClass::RunnerError,
                "primary code {code} must stay RunnerError",
            );
        }

        // Non-sqlx reasons keep the default classification so the
        // helper can be applied unconditionally at every emit site.
        assert_eq!(
            classify_runner_failure_reason("verify step 2 failed (exit 1)"),
            FailureClass::RunnerError,
        );
        assert_eq!(
            classify_runner_failure_reason("auto-bypass thrashing: trio docs failed"),
            FailureClass::RunnerError,
        );

        // A token shaped like the marker but not parseable as an
        // integer must not flip the classification. Guards against a
        // future log line that happens to contain the literal
        // "(code: " prose.
        assert_eq!(
            classify_runner_failure_reason("(code: not-a-number) sneaky"),
            FailureClass::RunnerError,
        );
    }

    /// Confirm the classifier also works against a real sqlx
    /// `Error::Database` formatted through Display — this is the
    /// surface the runner adapters actually emit when a store call
    /// fails inside `adapter.run` and the reason is `format!("{e}")`.
    /// Uses a tiny inline `DatabaseError` impl so the test does not
    /// have to provoke a live SQLITE_FULL on the filesystem.
    #[test]
    fn classify_runner_failure_reason_matches_sqlx_display() {
        use sqlx::error::{DatabaseError, ErrorKind};
        use std::borrow::Cow;
        use std::error::Error as StdError;
        use std::fmt;

        #[derive(Debug)]
        struct FakeSqliteError {
            code: i32,
        }

        impl fmt::Display for FakeSqliteError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Mirror sqlx-sqlite's Display verbatim: this is the
                // string the runner adapters end up wrapping in their
                // RunnerOutcome::Failed reason.
                write!(f, "(code: {}) database or disk is full", self.code)
            }
        }

        impl StdError for FakeSqliteError {}

        impl DatabaseError for FakeSqliteError {
            fn message(&self) -> &str {
                "database or disk is full"
            }
            fn code(&self) -> Option<Cow<'_, str>> {
                Some(format!("{}", self.code).into())
            }
            fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
                self
            }
            fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
                self
            }
            fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
                self
            }
            fn kind(&self) -> ErrorKind {
                ErrorKind::Other
            }
        }

        let err = sqlx::Error::Database(Box::new(FakeSqliteError { code: 13 }));
        let reason = format!("{err}");
        assert_eq!(
            classify_runner_failure_reason(&reason),
            FailureClass::InfrastructureError,
            "sqlx Display of SQLITE_FULL must round-trip through the reason mapper",
        );

        let err = sqlx::Error::Database(Box::new(FakeSqliteError { code: 19 }));
        let reason = format!("{err}");
        assert_eq!(
            classify_runner_failure_reason(&reason),
            FailureClass::RunnerError,
            "SQLITE_CONSTRAINT must stay RunnerError",
        );
    }

    #[test]
    fn stage_prompt_includes_goal_and_position() {
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[1], 2, None, None, None);
        assert!(prompt.contains("Stage 2 of 2"));
        assert!(prompt.contains("two"));
        assert!(prompt.contains("test goal"));
        assert!(!prompt.contains("REVIEW"));
    }

    #[test]
    fn stage_prompt_prepends_operator_comment_block_above_goal() {
        // Surface F thread-through: when the prior stage auto-bypassed
        // under a policy, the canned comment must appear above the
        // `# Job goal` heading so the model reads the operator's
        // pre-authorised guidance before anything else in the prompt.
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(
            planned[1],
            2,
            None,
            Some("Operator policy: Quick. ship it."),
            None,
        );
        let op_idx = prompt
            .find("# Operator comment")
            .expect("operator-comment heading missing");
        let goal_idx = prompt.find("# Job goal").expect("job-goal heading missing");
        assert!(
            op_idx < goal_idx,
            "operator-comment block must precede job goal; got prompt: {prompt}"
        );
        assert!(prompt.contains("Operator policy: Quick. ship it."));
    }

    /// Build an in-memory store seeded with one repo + one job under
    /// the caller-supplied auto-bypass policy, returning the persisted
    /// `JobId` so the test can build a `RunnerContext`. Keeps each
    /// classify_stage_failure test self-contained without dragging the
    /// session_idle fixtures into scope.
    async fn seed_store_with_policy(
        policy: Option<codeless_types::AutoBypassPolicy>,
        stop_reason: Option<codeless_types::StopReason>,
    ) -> (Arc<SqliteStore>, codeless_types::JobId) {
        use codeless_types::{
            CostCents, GitAuth, Job, JobId, JobStatus, RepoId, UnixMillis, WorkspaceMode,
        };
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrations::MIGRATOR.run(&pool).await.unwrap();
        let store = Arc::new(SqliteStore::new(pool));
        let repo = codeless_types::Repo {
            id: RepoId::new(),
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
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
            stop_reason,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "codeless/test".into(),
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
            auto_bypass_policy: policy,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        };
        let job_id = job.id;
        store.insert_job(&job).await.unwrap();
        (store, job_id)
    }

    async fn test_runner_context(job_id: codeless_types::JobId) -> RunnerContext {
        use crate::event_bus::EventBus;
        // The bus is unused by `classify_stage_failure` — it consults
        // the store only. Construct a minimal in-memory bus to satisfy
        // the `RunnerContext` field shape without a second pool.
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrations::MIGRATOR.run(&pool).await.unwrap();
        RunnerContext {
            job_id,
            stage_id: None,
            bus: Arc::new(EventBus::new(pool, 16)),
            worktree_path: None,
            cancel: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn classify_halts_when_no_policy_set() {
        let (store, job_id) = seed_store_with_policy(None, None).await;
        let ctx = test_runner_context(job_id).await;
        match classify_stage_failure(Some(store.as_ref()), &ctx).await {
            FailureAction::Halt => {}
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classify_auto_bypasses_under_policy_when_no_cap_breach() {
        let (store, job_id) =
            seed_store_with_policy(Some(codeless_types::AutoBypassPolicy::Quick), None).await;
        let ctx = test_runner_context(job_id).await;
        match classify_stage_failure(Some(store.as_ref()), &ctx).await {
            FailureAction::AutoBypass {
                policy_name,
                comment,
                thrash_guard_applies,
            } => {
                assert_eq!(policy_name, "Quick");
                assert!(
                    comment.starts_with("Operator policy: Quick."),
                    "unexpected canned comment: {comment}"
                );
                // Every preset other than Relentless leaves the
                // two-strike guard armed.
                assert!(thrash_guard_applies);
            }
            other => panic!("expected AutoBypass, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classify_under_relentless_opts_out_of_thrash_guard() {
        // AUTO-BYPASS-DECISIONS.md Q7: the only policy variant where
        // `thrash_guard_applies` is false. The runner reads this flag
        // before consulting the guard; the unit test pins the flag at
        // the classification seam so a future refactor cannot drop it.
        let (store, job_id) =
            seed_store_with_policy(Some(codeless_types::AutoBypassPolicy::Relentless), None).await;
        let ctx = test_runner_context(job_id).await;
        match classify_stage_failure(Some(store.as_ref()), &ctx).await {
            FailureAction::AutoBypass {
                policy_name,
                thrash_guard_applies,
                ..
            } => {
                assert_eq!(policy_name, "Relentless");
                assert!(!thrash_guard_applies);
            }
            other => panic!("expected AutoBypass, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classify_halts_on_cost_cap_breach_even_with_policy() {
        // Cap breach must win — DOCS/AUTO-BYPASS-DECISIONS.md Q1 fences
        // CostCap / WallClock off from the policy unconditionally.
        let (store, job_id) = seed_store_with_policy(
            Some(codeless_types::AutoBypassPolicy::JustCode),
            Some(codeless_types::StopReason::CostCap),
        )
        .await;
        let ctx = test_runner_context(job_id).await;
        match classify_stage_failure(Some(store.as_ref()), &ctx).await {
            FailureAction::Halt => {}
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classify_halts_when_context_already_cancelled() {
        // External stop_job / pause_job is observed via the cancel
        // token; auto-bypass must not re-interpret it as "retry".
        let (store, job_id) =
            seed_store_with_policy(Some(codeless_types::AutoBypassPolicy::Cheap), None).await;
        let mut ctx = test_runner_context(job_id).await;
        ctx.cancel = CancellationToken::new();
        ctx.cancel.cancel();
        match classify_stage_failure(Some(store.as_ref()), &ctx).await {
            FailureAction::Halt => {}
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[test]
    fn stage_prompt_omits_operator_block_when_no_comment_supplied() {
        // The default (no auto-bypass) path must not emit a stray
        // `# Operator comment` heading — only a stage that follows a
        // policy-triggered auto-bypass should see that envelope.
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[1], 2, None, None, None);
        assert!(!prompt.contains("# Operator comment"));
    }

    #[test]
    fn with_pending_operator_comment_drops_empty_string() {
        // Slack's parser cannot prove a quoted argument is non-empty
        // before the RPC fires; the runner's builder normalises an
        // empty string back to `None` so the first stage's prompt
        // does not grow a stray empty `# Operator comment` heading.
        let r = TemplateRunner::new(template_with_stages(&["one"]))
            .with_pending_operator_comment(Some(String::new()));
        assert!(r.pending_operator_comment.is_none());
        let r2 = TemplateRunner::new(template_with_stages(&["one"]))
            .with_pending_operator_comment(Some("ship it".into()));
        assert_eq!(r2.pending_operator_comment.as_deref(), Some("ship it"));
    }

    #[test]
    fn pending_operator_comment_seeds_first_stage_prompt() {
        // The Slack `resume <job-id> "<comment>"` path threads its
        // free-text payload through `pending_operator_comment`. On
        // entry to `run`, that value seeds `next_stage_prefix`, which
        // `stage_prompt` then renders into the `# Operator comment`
        // block above the goal. This test pins the contract by
        // calling `stage_prompt` with the same value the runner
        // would pass on the first iteration; the `stage_prompt`
        // assembly path is shared with the auto-bypass thread-through
        // and is already covered for the empty-comment case above.
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]))
            .with_pending_operator_comment(Some("ship it".into()));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(
            planned[0],
            2,
            None,
            r.pending_operator_comment.as_deref(),
            None,
        );
        let op_idx = prompt
            .find("# Operator comment")
            .expect("operator-comment heading missing on first-stage prompt");
        let goal_idx = prompt.find("# Job goal").expect("job-goal heading missing");
        assert!(
            op_idx < goal_idx,
            "operator-comment block must precede job goal; got prompt: {prompt}"
        );
        assert!(prompt.contains("ship it"));
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
        let prompt = r.stage_prompt(planned[0], 2, None, None, None);
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
            raw_tail: None,
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
    async fn evaluate_review_gate_missing_sentinel_on_substantive_handover_is_lenient_pass() {
        // The lenient parser was added because production-realistic
        // agents forget the sentinel grammar but still do the work.
        // A handover with real Done + Next content now PASSes
        // implicitly; the gate's audit log records the lenient
        // reason via tracing::warn (asserted indirectly via the
        // verdict shape).
        use codeless_types::{Handover, JobId};
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let h = Handover {
            done: vec!["forgot the sentinel".into()],
            next: vec!["next step".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .unwrap();

        let verdict = evaluate_review_gate(Some(tmp.path()), job_id, stage_id)
            .await
            .expect("lenient parser implicit-PASSes a substantive handover");
        match verdict {
            ReviewVerdict::Pass { reason } => assert!(
                reason.contains("implicit"),
                "implicit reason expected, got: {reason}"
            ),
            other => panic!("expected lenient Pass, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn evaluate_review_gate_empty_handover_still_fails() {
        // The lenient parser only PASSes when Done AND Next have
        // substance. A handover with placeholder bullets ("(none)")
        // is the "model died mid-stream" failure mode; the gate
        // must still halt. Bypass the runtime's write-time
        // validator (which refuses empty Done/Next) by writing the
        // markdown directly — the on-disk shape is what the parser
        // sees, and a model that emitted only placeholders is the
        // case under test.
        use codeless_types::JobId;
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let path = crate::handover::handover_path(tmp.path(), job_id, stage_id);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            "## Done\n\n- (none)\n\n## Next\n\n- (none)\n\n## What you need to know\n\n- (none)\n\n## Open questions\n\n- (none)\n",
        )
        .await
        .unwrap();

        let err = evaluate_review_gate(Some(tmp.path()), job_id, stage_id)
            .await
            .expect_err("genuinely empty handover is an error");
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
        let p1 = r.stage_prompt(planned[0], 2, Some(tmp.path()), None, None);
        assert!(
            p1.contains("# Stage 1 docs"),
            "missing stage-docs heading: {p1}"
        );
        assert!(p1.contains("ROUTING DOC BODY"));
        assert!(!p1.contains("HANDLERS DOC BODY"));

        // Stage 2 sees handlers.md, not routing.md.
        let p2 = r.stage_prompt(planned[1], 2, Some(tmp.path()), None, None);
        assert!(p2.contains("# Stage 2 docs"));
        assert!(p2.contains("HANDLERS DOC BODY"));
        assert!(!p2.contains("ROUTING DOC BODY"));
    }

    #[test]
    fn stage_prompt_omits_docs_block_when_stage_has_none() {
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let prompt = r.stage_prompt(planned[0], 2, None, None, None);
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
            PreCheckOutcome::Pass { .. } => {}
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
            PreCheckOutcome::Fail { reason, missing } => {
                assert!(
                    reason.contains("unrelated/notes.md"),
                    "reason did not name the missing path: {reason}"
                );
                assert!(
                    !reason.contains("a/b.rs"),
                    "reason should not list verified paths as missing: {reason}"
                );
                assert!(
                    missing.iter().any(|m| m == "unrelated/notes.md"),
                    "missing list did not include the claimed path: {missing:?}"
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
    async fn precheck_reports_nothing_to_verify_when_done_names_no_paths() {
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
            PreCheckOutcome::NothingToVerify => {}
            other => panic!("expected NothingToVerify, got {other:?}"),
        }
    }

    /// Per-stage persona override (D1): when the runner walks a
    /// template whose stage carries `persona: <id>`, the published
    /// `StageStarted` envelope must echo that id so the recorder can
    /// stamp `stages.persona_id`. A stage with no override emits
    /// `persona_id = None`, meaning "inherit the job-level persona".
    #[tokio::test]
    async fn stage_started_event_carries_per_stage_persona() {
        use crate::event_bus::SubscribeFilter;
        use crate::rpc::InProcessRpc;
        use crate::runner::{Runner, RunnerContext};
        use codeless_types::{Event, JobId};
        use futures_util::StreamExt;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let src = r#"
name: t
goal: test goal
stages:
  - title: implement
    persona: "builtin:coder"
  - title: ship
"#;
        let template = JobTemplate::parse_yaml(src).unwrap();
        // `use_mock_runner` avoids touching the claude binary; the
        // recorder hookup is exercised by the stage_recorder tests.
        let runner = TemplateRunner::new(template).with_mock_runner();
        let rpc = InProcessRpc::new().await.unwrap();
        let bus = rpc.bus().clone();
        let mut stream = bus
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
        runner.run(ctx).await;

        let mut seen = Vec::new();
        while let Ok(Some(env)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
        {
            let env = env.unwrap();
            if let Event::StageStarted {
                ordinal,
                persona_id,
                ..
            } = env.event
            {
                seen.push((ordinal, persona_id));
            }
        }
        assert_eq!(
            seen,
            vec![(0, Some("builtin:coder".to_string())), (1, None)],
            "stage 0 carries its override; stage 1 inherits",
        );
    }

    #[test]
    fn stage_prompt_prepends_prior_failure_block_above_operator_and_goal() {
        // The prior-failure block (authored by
        // `build_prior_failure_context`) is the first thing the model
        // sees when an ordinal is being re-attempted. It must sit
        // above the operator-comment block (which is in turn above
        // the goal) so the read order is: what went wrong → operator
        // guidance → goal.
        let r = TemplateRunner::new(template_with_stages(&["one", "two"]));
        let planned = r.template.planned_stages();
        let block = "# Prior attempt failed (stage 1)\n\nhandover claimed paths missing\n";
        let prompt = r.stage_prompt(planned[0], 2, None, Some("ship it"), Some(block));
        let pf = prompt
            .find("# Prior attempt failed")
            .expect("prior-failure heading missing");
        let op = prompt
            .find("# Operator comment")
            .expect("operator-comment heading missing");
        let goal = prompt.find("# Job goal").expect("job-goal heading missing");
        assert!(
            pf < op && op < goal,
            "ordering must be prior-failure < operator < goal; got prompt: {prompt}"
        );
    }

    #[tokio::test]
    async fn build_prior_failure_context_extracts_review_diagnostics() {
        // Pin the wire-level contract that consumes the same
        // `events` rows the UI renders: pre-check `missing`, verdict
        // `reason`, stage-completed `reason`. A missing field
        // degrades to a partial block; an empty events query
        // degrades to `None` (no empty heading).
        use crate::store::SqliteStore;
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let store = SqliteStore::new(pool.clone());
        let stage_id = StageId::new();
        let job_id = codeless_types::JobId::new();
        let now = now_ms();
        // Insert three events for the same stage_id.
        for (ty, payload) in [
            (
                "review-pre-check",
                serde_json::json!({
                    "stage_id": stage_id.to_string(),
                    "outcome": {"outcome": "fail", "missing": ["foo.rs", "bar.rs"]}
                }),
            ),
            (
                "review-verdict",
                serde_json::json!({
                    "stage_id": stage_id.to_string(),
                    "verdict": {"verdict": "auto-fail", "reason": "claimed paths absent"}
                }),
            ),
            (
                "stage-completed",
                serde_json::json!({"stage_id": stage_id.to_string(), "status": "failed"}),
            ),
        ] {
            sqlx::query(
                "INSERT INTO events (job_id, stage_id, type, payload, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(job_id.to_string())
            .bind(stage_id.to_string())
            .bind(ty)
            .bind(payload.to_string())
            .bind(now.0)
            .execute(&pool)
            .await
            .unwrap();
        }
        let block = build_prior_failure_context(&store, stage_id, 6)
            .await
            .unwrap()
            .expect("expected a block from three diagnostic events");
        assert!(block.contains("# Prior attempt failed (stage 7)"));
        assert!(block.contains("foo.rs"));
        assert!(block.contains("bar.rs"));
        assert!(block.contains("claimed paths absent"));
        // Empty stage_id (no events) returns None so the runner does
        // not stamp an empty heading.
        let empty = build_prior_failure_context(&store, StageId::new(), 0)
            .await
            .unwrap();
        assert!(empty.is_none());
    }

    #[tokio::test]
    async fn template_runner_emits_trio_after_stage_started() {
        // The closing trio is runtime-injected at stage entry so a
        // misbehaving runner cannot silently skip the closing three
        // steps. This test pins the event order on the wire:
        // `StageStarted` first, then exactly one `TodoAdded` per trio
        // kind (`Checks`, `Docs`, `Git`) in that order, per stage.
        use crate::event_bus::SubscribeFilter;
        use crate::rpc::InProcessRpc;
        use crate::runner::{Runner, RunnerContext};
        use codeless_types::{Event, JobId, TodoKind};
        use futures_util::StreamExt;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let runner = TemplateRunner::new(template_with_stages(&["one", "two"])).with_mock_runner();
        let rpc = InProcessRpc::new().await.unwrap();
        let bus = rpc.bus().clone();
        let mut stream = bus
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
        runner.run(ctx).await;

        let mut order: Vec<&'static str> = Vec::new();
        let mut trio_kinds: Vec<TodoKind> = Vec::new();
        while let Ok(Some(env)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
        {
            let env = env.unwrap();
            match env.event {
                Event::StageStarted { .. } => order.push("stage-started"),
                Event::TodoAdded { kind, .. } => {
                    order.push("todo-added");
                    trio_kinds.push(kind);
                }
                _ => {}
            }
        }

        // Two stages, each emits one StageStarted followed immediately
        // by three TodoAdded envelopes — six trio rows total.
        let stage_started_count = order.iter().filter(|t| **t == "stage-started").count();
        let todo_added_count = order.iter().filter(|t| **t == "todo-added").count();
        assert_eq!(stage_started_count, 2);
        assert_eq!(todo_added_count, 6);
        assert_eq!(
            trio_kinds,
            vec![
                TodoKind::Checks,
                TodoKind::Docs,
                TodoKind::Git,
                TodoKind::Checks,
                TodoKind::Docs,
                TodoKind::Git,
            ]
        );

        // First emission per stage: stage-started precedes its trio.
        let first_started = order
            .iter()
            .position(|t| *t == "stage-started")
            .expect("at least one stage-started");
        assert!(order[first_started + 1..first_started + 4]
            .iter()
            .all(|t| *t == "todo-added"));
    }

    #[tokio::test]
    async fn template_runner_blocks_stage_completed_until_trio_resolved() {
        // The runtime refuses to publish `StageCompleted{ Passed }`
        // until the trio is `Done`/`Skipped` on the stage's terminal
        // task. This test installs the recorder so the bus's
        // `TodoAdded` envelopes land in SQLite, runs one mock stage,
        // confirms `StageCompleted` is held back, then resolves the
        // trio out-of-band and asserts the gate releases.
        use crate::event_bus::SubscribeFilter;
        use crate::rpc::InProcessRpc;
        use crate::runner::{Runner, RunnerContext};
        use codeless_types::{Event, JobStatus, TodoStatus, WorkspaceMode};
        use futures_util::StreamExt;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let rpc = InProcessRpc::new().await.unwrap();
        let bus = rpc.bus().clone();
        let store = rpc.store().clone();

        // Recorder consumes the bus and persists the trio rows so the
        // gate can find them via `trio_resolved`.
        let recorder = crate::stage_recorder::spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

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
            status: JobStatus::Running,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: codeless_types::CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: codeless_types::CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let runner = TemplateRunner::new(template_with_stages(&["only"]))
            .with_mock_runner()
            .with_store(store.clone());

        let ctx = RunnerContext {
            job_id: job.id,
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let cancel_for_test = ctx.cancel.clone();
        let run_handle = tokio::spawn(async move { runner.run(ctx).await });

        // Tail the bus and wait for the trio to appear. The recorder
        // persists each `TodoAdded` and the gate then polls
        // `trio_resolved`; without external resolution the run hangs
        // here, which is the contract this test pins.
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();

        // Pull events from the bus until the third trio TodoAdded
        // (`Git`) lands — at that point the recorder has the rows it
        // needs to answer `trio_resolved`, and the runner is blocked
        // at the gate. Track whether StageCompleted has shown up; it
        // must not.
        let mut saw_stage_completed = false;
        let mut task_id: Option<TaskId> = None;
        let mut trio_seen = 0;
        let deadline_inject = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while trio_seen < 3 && std::time::Instant::now() < deadline_inject {
            if let Ok(Some(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(150), sub.next()).await
            {
                let env = env.unwrap();
                match env.event {
                    Event::TodoAdded { task_id: t, .. } => {
                        task_id = Some(t);
                        trio_seen += 1;
                    }
                    Event::StageCompleted { .. } => saw_stage_completed = true,
                    _ => {}
                }
            }
        }
        assert_eq!(trio_seen, 3, "three trio TodoAdded envelopes must arrive");
        let task_id = task_id.expect("trio carries task_id");
        // Brief settle so the recorder finishes persisting before we
        // poll the rows out. The recorder is a separate task on the
        // same bus.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(
            !saw_stage_completed,
            "StageCompleted must not be emitted while trio is unresolved"
        );

        let todos = store.list_todos_for_task(task_id).await.unwrap();
        let trio: Vec<_> = todos.iter().filter(|t| t.kind.is_trio()).collect();
        assert_eq!(trio.len(), 3, "three trio rows must be persisted");
        for t in &trio {
            store
                .update_todo_status(t.id, TodoStatus::Done, now_ms(), None)
                .await
                .unwrap();
        }

        // Gate releases on next poll cycle; await StageCompleted.
        let mut released = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(150), sub.next()).await
            {
                let env = env.unwrap();
                if matches!(
                    env.event,
                    Event::StageCompleted {
                        status: StageStatus::Passed,
                        ..
                    }
                ) {
                    released = true;
                    break;
                }
            }
        }
        assert!(released, "gate must release once trio rows are resolved");

        // Hand back the join handle: drop the cancel/recorder cleanly.
        let _ = run_handle.await;
        let _ = cancel_for_test;
        recorder.abort();
    }

    #[tokio::test]
    async fn no_verify_stage_resolves_trio_via_skip_and_releases_gate() {
        // Regression for DOCS/sessions/2026-05-18-trio-gate-wiring-fix.md:
        // a stage with no `verify:` block, on the mock runner (no
        // `Docs` rail), and no worktree path (no `Git` commit) used
        // to hang at the trio gate forever because no production
        // caller drove the `Checks` or `Git` rails to a terminal
        // state. With the wiring fix in place, the skip-pending
        // helper flips every still-`Pending` rail to `Skipped` right
        // before the gate, so the stage completes within a poll
        // cycle. Pre-fix this test hangs and trips the timeout below.
        use crate::event_bus::SubscribeFilter;
        use crate::rpc::InProcessRpc;
        use crate::runner::{Runner, RunnerContext};
        use codeless_types::{Event, JobStatus, WorkspaceMode};
        use futures_util::StreamExt;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let rpc = InProcessRpc::new().await.unwrap();
        let bus = rpc.bus().clone();
        let store = rpc.store().clone();
        let recorder = crate::stage_recorder::spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

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
            status: JobStatus::Running,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: codeless_types::CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: codeless_types::CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let runner = TemplateRunner::new(template_with_stages(&["only"]))
            .with_mock_runner()
            .with_store(store.clone());
        let ctx = RunnerContext {
            job_id: job.id,
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();
        // Bound the run so a regression hangs the test instead of the
        // whole test binary; pre-fix this expires.
        let run = tokio::time::timeout(std::time::Duration::from_secs(5), runner.run(ctx))
            .await
            .expect("template runner must finish; pre-fix the trio gate held it forever");
        assert_eq!(run, crate::runner::RunnerOutcome::Completed);

        let mut saw_passed = false;
        while let Ok(Some(env)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.next()).await
        {
            if let Event::StageCompleted {
                status: StageStatus::Passed,
                ..
            } = env.unwrap().event
            {
                saw_passed = true;
                break;
            }
        }
        assert!(
            saw_passed,
            "StageCompleted{{ Passed }} must publish after skip_pending_trio_rows resolves the rails"
        );
        recorder.abort();
    }

    #[tokio::test]
    async fn failed_trio_row_routes_stage_to_failed_with_detail() {
        // Regression test for job `01KRX4ZPF...`: when a trio row
        // (here `Docs`) lands `Failed`, the gate must publish
        // `StageCompleted { Failed }` with the per-rail failure_detail
        // surfaced into the stage's `failure_detail`. The previous
        // boolean gate hung forever on the same input.
        use crate::event_bus::SubscribeFilter;
        use crate::rpc::InProcessRpc;
        use crate::runner::{Runner, RunnerContext};
        use codeless_types::{Event, JobStatus, TodoKind, TodoStatus, WorkspaceMode};
        use futures_util::StreamExt;
        use std::sync::Arc;
        use tokio_util::sync::CancellationToken;

        let rpc = InProcessRpc::new().await.unwrap();
        let bus = rpc.bus().clone();
        let store = rpc.store().clone();

        let recorder = crate::stage_recorder::spawn_stage_recorder(bus.clone(), store.clone())
            .await
            .unwrap();

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
            status: JobStatus::Running,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("p".into()),
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: codeless_types::CostCents::ZERO,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            cost_cents: codeless_types::CostCents::ZERO,
            started_at: None,
            ended_at: None,
            created_at: now_ms(),
        };
        store.insert_job(&job).await.unwrap();

        let runner = TemplateRunner::new(template_with_stages(&["only"]))
            .with_mock_runner()
            .with_store(store.clone());

        let ctx = RunnerContext {
            job_id: job.id,
            stage_id: None,
            bus: Arc::clone(&bus),
            worktree_path: None,
            cancel: CancellationToken::new(),
        };
        let run_handle = tokio::spawn(async move { runner.run(ctx).await });

        let mut sub = bus
            .subscribe_since(SubscribeFilter::All, None)
            .await
            .unwrap();

        let mut task_id: Option<TaskId> = None;
        let mut trio_seen = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while trio_seen < 3 && std::time::Instant::now() < deadline {
            if let Ok(Some(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(150), sub.next()).await
            {
                if let Event::TodoAdded { task_id: t, .. } = env.unwrap().event {
                    task_id = Some(t);
                    trio_seen += 1;
                }
            }
        }
        let task_id = task_id.expect("trio carries task_id");
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        // Resolve checks/git cleanly, fail docs with a reason. The
        // gate must surface the reason in `failure_detail` rather than
        // wait for docs to flip to `Done`.
        let todos = store.list_todos_for_task(task_id).await.unwrap();
        for t in todos.iter().filter(|t| t.kind.is_trio()) {
            let (status, reason): (TodoStatus, Option<&str>) = match t.kind {
                TodoKind::Docs => (TodoStatus::Failed, Some("write handover: disk full")),
                _ => (TodoStatus::Done, None),
            };
            store
                .update_todo_status(t.id, status, now_ms(), reason)
                .await
                .unwrap();
        }

        let mut saw_failed = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(150), sub.next()).await
            {
                if let Event::StageCompleted {
                    status: StageStatus::Failed,
                    failure_detail,
                    ..
                } = env.unwrap().event
                {
                    let detail = failure_detail.unwrap_or_default();
                    assert!(
                        detail.contains("docs"),
                        "stage failure_detail must name the failed rail; got {detail:?}"
                    );
                    assert!(
                        detail.contains("disk full"),
                        "stage failure_detail must include the per-rail reason; got {detail:?}"
                    );
                    saw_failed = true;
                    break;
                }
            }
        }
        assert!(
            saw_failed,
            "trio gate must emit StageCompleted{{ Failed }} on a failed trio rail"
        );

        let _ = run_handle.await;
        recorder.abort();
    }
}
