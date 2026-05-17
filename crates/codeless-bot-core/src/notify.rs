//! Renderers for outbound failure notifications. One function per
//! event kind the publisher fires on: [`format_job_failed`] for
//! `Event::JobFailed`, [`format_job_stopped`] for `Event::JobStopped`.
//! Both produce a single plain-text block matching the Surface 1
//! mockup from `.codeless/jobs/slack-integration/SCOPE.md`:
//!
//! ```text
//! [!] Job "scope-mutable-ui" - Failed at stage 8/13
//!     Stage:  "REVIEW after per-job action loop"
//!     Reason: <stop_reason or "failed">
//!     Cost:   $52.64 / $150.00 cap
//!     Reply in this thread: resume bypass | resume "<comment>" | stop
//! ```
//!
//! When the failing stage is a REVIEW gate the publisher passes in a
//! [`ReviewContext`] captured from the bus (`ReviewPreCheck` and
//! `ReviewVerdict` events). The renderer then appends the Surface 2
//! structured block — gate type, the path list from a diff-verify
//! pre-check fail, and the verdict reason text — matching the
//! Surface 2 mockup in the same SCOPE doc:
//!
//! ```text
//! [!] Job "scope-mutable-ui" - Failed at stage 8/13
//!     Stage:  "REVIEW after per-job action loop"
//!     Type:   diff-verify pre-check auto-fail
//!     Missing paths:
//!       - DOCS/SCOPE-MUTABLE-UI.md
//!     Verdict: handover claims paths not in the diff
//!     Cost:   $52.64 / $150.00 cap
//!     Reply in this thread: resume bypass | resume "<comment>" | stop
//! ```
//!
//! The mockup in the SCOPE doc uses an emoji glyph (`U+1F6A8`); the
//! repo-wide rule (CLAUDE.md R2) forbids them, so the renderer uses
//! the same `[!]` tag the inbound-reply path settled on for warnings
//! / alerts. The tag carries the same meaning — "something needs
//! operator attention" — so a Slack reader who already knows the
//! reply-path conventions reads the outbound shape the same way.
//!
//! Renderers stay pure: they take the inputs already resolved by
//! the publisher (job row, optional stage rollup with the ordinal
//! and title, optional review context) and produce a string. Network
//! access and event-bus filtering live next door in [`crate::outbound`];
//! keeping the split lets the formatting be unit-tested without
//! spawning a Slack stub.

use codeless_rpc::methods::StageRollup;
use codeless_types::review_gate::{PreCheckOutcome, ReviewVerdict};
use codeless_types::{Job, StopReason};

/// Per-stage REVIEW-gate context captured by the outbound publisher
/// from `ReviewPreCheck` / `ReviewVerdict` events on the bus. Both
/// fields are independently optional: a model-driven `Fail` arrives
/// as a `ReviewVerdict` with no prior `ReviewPreCheck::Fail`; a
/// pre-check auto-fail arrives as both, with the pre-check ahead of
/// the verdict; a `Skipped` / `NothingToVerify` pre-check sets only
/// the pre-check field. Renderers treat absence as "do not render
/// that line"; both absent means the failing stage was not a REVIEW
/// gate and the structured block collapses entirely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewContext {
    pub pre_check: Option<PreCheckOutcome>,
    pub verdict: Option<ReviewVerdict>,
}

impl ReviewContext {
    /// True when neither field has been observed yet — the publisher
    /// treats this case the same as not having a context at all and
    /// skips passing it to the renderer.
    pub fn is_empty(&self) -> bool {
        self.pre_check.is_none() && self.verdict.is_none()
    }
}

use crate::reply::template_name;

/// Format a `JobFailed` notification. `stage` is the failing stage
/// rollup (`None` when the publisher's `list_stages` call returned
/// nothing — the notification still goes out, just without the
/// stage-N/N header). `total_stages` is the count of stages on the
/// job (also `None` when the call failed); the header degrades to
/// `"stage 8"` when only one of the two is available, and to a bare
/// `"Failed"` when neither is. `review` is the REVIEW-gate context
/// captured by the publisher from the bus; `None` (or
/// [`ReviewContext::is_empty`]) collapses the Surface 2 block so a
/// non-REVIEW failure renders the bare Surface 1 shape.
pub fn format_job_failed(
    job: &Job,
    stage: Option<&StageRollup>,
    total_stages: Option<u32>,
    review: Option<&ReviewContext>,
) -> String {
    render(job, stage, total_stages, OutboundEvent::Failed, review)
}

/// Format a `JobStopped` notification. Shape mirrors `format_job_failed`;
/// the header reads `Stopped (<reason>)` instead, since `JobStopped`
/// carries the reason on the event itself and the verb difference is
/// the operator's main "do I need to act" signal.
pub fn format_job_stopped(
    job: &Job,
    stage: Option<&StageRollup>,
    total_stages: Option<u32>,
    reason: StopReason,
    review: Option<&ReviewContext>,
) -> String {
    render(
        job,
        stage,
        total_stages,
        OutboundEvent::Stopped(reason),
        review,
    )
}

enum OutboundEvent {
    Failed,
    Stopped(StopReason),
}

fn render(
    job: &Job,
    stage: Option<&StageRollup>,
    total_stages: Option<u32>,
    kind: OutboundEvent,
    review: Option<&ReviewContext>,
) -> String {
    let name = template_name(job).unwrap_or_else(|| "(no-template)".to_string());
    let verb = match kind {
        OutboundEvent::Failed => "Failed",
        OutboundEvent::Stopped(_) => "Stopped",
    };
    let header = match (stage.map(|s| s.stage.ordinal), total_stages) {
        // Stage ordinals are 0-based on the wire; the operator-facing
        // counter is 1-based ("stage 8/13") because every other
        // surface (web UI, run log, SCOPE mockup) uses 1-based stage
        // numbers. Render that consistently here.
        (Some(ord), Some(total)) => format!("{verb} at stage {}/{}", ord + 1, total),
        (Some(ord), None) => format!("{verb} at stage {}", ord + 1),
        (None, _) => verb.to_string(),
    };
    let mut out = format!("[!] Job \"{name}\" - {header}\n");
    if let Some(s) = stage {
        out.push_str(&format!("    Stage:  \"{}\"\n", s.stage.name));
    }
    // The Surface 2 block sits between the stage line and the reason
    // line so the operator reads gate-specific context first when
    // it's available — the bare stop_reason word ("runner crashed",
    // "cost-cap exceeded") is the right summary for a non-REVIEW
    // failure, but a REVIEW auto-fail's reason word is always
    // `runner crashed` / `None` and the gate-type + verdict text
    // carries the information the operator actually needs.
    let review_nonempty = review.filter(|r| !r.is_empty());
    if let Some(r) = review_nonempty {
        render_review_block(&mut out, r);
    }
    // Failure reason: `JobFailed` does not carry one on the event, so
    // fall back to the job row's `stop_reason` (the runtime stamps it
    // before publishing). `JobStopped` carries the reason directly;
    // prefer that over the row value (which may be the same, but the
    // event field is the wire-level source of truth and avoids a race
    // between the publish and the row update for callers that haven't
    // yet observed the post-publish refresh).
    let reason_word = match kind {
        OutboundEvent::Failed => job.stop_reason.map(stop_reason_word),
        OutboundEvent::Stopped(r) => Some(stop_reason_word(r)),
    };
    if let Some(word) = reason_word {
        out.push_str(&format!("    Reason: {word}\n"));
    }
    let cost = (job.cost_cents.as_i64() as f64) / 100.0;
    let cap = (job.cost_cap_cents.as_i64() as f64) / 100.0;
    out.push_str(&format!("    Cost:   ${cost:.2} / ${cap:.2} cap\n"));
    out.push_str("    Reply in this thread: resume bypass | resume \"<comment>\" | stop");
    out
}

/// Append the Surface 2 REVIEW-gate block to the notification body.
/// The block has three optional lines whose presence depends on what
/// the publisher captured from the bus:
///
///   - `Type:` — the gate's failure shape. Derived from the pair of
///     (pre-check outcome, verdict variant) so the operator sees
///     "diff-verify pre-check auto-fail" vs "model rejected handover"
///     vs "runtime auto-fail (sentinel)" at a glance.
///   - `Missing paths:` — one bullet per path the pre-check rejected.
///     Only present for `PreCheckOutcome::Fail`. Surfaces the exact
///     list rather than a count so a decision to bypass / re-prompt
///     can be made from the notification alone (Surface 2's SCOPE
///     mockup shows the path list verbatim).
///   - `Verdict:` — the reason text from the model or the runtime's
///     auto-fail. Sourced from `ReviewVerdict::{Fail,AutoFail,Pass}`.
///     Pass is included for the rare case where the runtime fired a
///     JobFailed *after* a passing REVIEW (the verdict is still
///     informative for the operator).
fn render_review_block(out: &mut String, review: &ReviewContext) {
    if let Some(label) = review_type_label(review) {
        out.push_str(&format!("    Type:   {label}\n"));
    }
    if let Some(PreCheckOutcome::Fail { missing }) = &review.pre_check {
        if !missing.is_empty() {
            out.push_str("    Missing paths:\n");
            for path in missing {
                out.push_str(&format!("      - {path}\n"));
            }
        }
    }
    if let Some(verdict) = &review.verdict {
        let reason = match verdict {
            ReviewVerdict::Pass { reason }
            | ReviewVerdict::Fail { reason }
            | ReviewVerdict::AutoFail { reason } => reason,
        };
        if !reason.is_empty() {
            out.push_str(&format!("    Verdict: {reason}\n"));
        }
    }
}

/// Derive a one-line gate-type tag from the (pre-check, verdict) pair.
/// Returns `None` when the context carries nothing the operator can
/// meaningfully label (e.g. only a `Pass` pre-check and no verdict —
/// the gate did not fail, the failure must lie elsewhere on the
/// stage).
fn review_type_label(review: &ReviewContext) -> Option<&'static str> {
    match (&review.pre_check, &review.verdict) {
        (Some(PreCheckOutcome::Fail { .. }), Some(ReviewVerdict::AutoFail { .. })) => {
            Some("diff-verify pre-check auto-fail")
        }
        (Some(PreCheckOutcome::Fail { .. }), _) => Some("diff-verify pre-check failed"),
        (_, Some(ReviewVerdict::Fail { .. })) => Some("model rejected handover"),
        (_, Some(ReviewVerdict::AutoFail { .. })) => Some("runtime auto-fail"),
        (_, Some(ReviewVerdict::Pass { .. })) => Some("review passed (failure elsewhere)"),
        (Some(PreCheckOutcome::Skipped), None) => Some("diff-verify pre-check skipped"),
        (Some(PreCheckOutcome::NothingToVerify), None) => Some("nothing to verify"),
        (Some(PreCheckOutcome::Pass { .. }), None) => None,
        (None, None) => None,
    }
}

fn stop_reason_word(r: StopReason) -> &'static str {
    match r {
        StopReason::User => "user-stopped",
        StopReason::CostCap => "cost-cap exceeded",
        StopReason::WallClock => "wall-clock exceeded",
        StopReason::RunnerCrash => "runner crashed",
        StopReason::AutoBypassThrashing => "auto-bypass thrashing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::methods::StageRollup;
    use codeless_types::{
        CostCents, JobId, JobStatus, RepoId, Stage, StageId, StageStatus, WorkspaceMode,
    };

    fn sample_job(name: &str) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status: JobStatus::Failed,
            stop_reason: None,
            template_yaml: Some(format!("name: {name}\nstages: []\n")),
            prompt: None,
            runner: "claude".to_string(),
            branch: "codeless/x".to_string(),
            workspace_mode: WorkspaceMode::InRepo,
            worktree_path: None,
            cost_cap_cents: CostCents(15000),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(5264),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            started_at: None,
            ended_at: None,
            created_at: codeless_types::time::UnixMillis(0),
        }
    }

    fn sample_stage(ordinal: u32, name: &str, status: StageStatus) -> StageRollup {
        StageRollup {
            stage: Stage {
                id: StageId::new(),
                job_id: JobId::new(),
                ordinal,
                name: name.to_string(),
                status,
                verify_cmd: None,
                started_at: None,
                ended_at: None,
                session_id: None,
                goal: None,
                acceptance: None,
                last_activity_at: None,
                archived: false,
                persona_id: None,
                bypassed_at: None,
                bypassed_reason: None,
            },
            cost_cents: 0,
            task_count: 0,
        }
    }

    #[test]
    fn failed_with_full_context_renders_mockup_shape() {
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(7, "REVIEW after per-job action loop", StageStatus::Failed);
        let body = format_job_failed(&job, Some(&stage), Some(13), None);
        assert!(body.starts_with("[!] Job \"scope-mutable-ui\" - Failed at stage 8/13"));
        assert!(body.contains("Stage:  \"REVIEW after per-job action loop\""));
        assert!(body.contains("Reason: runner crashed"));
        assert!(body.contains("Cost:   $52.64 / $150.00 cap"));
        assert!(
            body.contains("Reply in this thread: resume bypass | resume \"<comment>\" | stop"),
            "missing reply hint in:\n{body}",
        );
        // The Surface 2 block is opt-in via the review context; a bare
        // `None` here must NOT introduce any of the structured lines.
        assert!(!body.contains("Type:"));
        assert!(!body.contains("Missing paths:"));
        assert!(!body.contains("Verdict:"));
    }

    #[test]
    fn failed_without_stage_rollup_degrades_to_bare_header() {
        let mut job = sample_job("smscope-smoke");
        job.stop_reason = Some(StopReason::CostCap);
        let body = format_job_failed(&job, None, None, None);
        assert!(body.starts_with("[!] Job \"smscope-smoke\" - Failed\n"));
        // The stage line is omitted entirely so a renderer fallback
        // does not invent a stage that does not exist on the job.
        assert!(!body.contains("Stage:"));
        assert!(body.contains("Reason: cost-cap exceeded"));
    }

    #[test]
    fn failed_with_ordinal_but_no_total_renders_partial_header() {
        let mut job = sample_job("hello-gin");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(2, "step three", StageStatus::Failed);
        let body = format_job_failed(&job, Some(&stage), None, None);
        // The total is unknown but the ordinal is — render the partial
        // header rather than dropping the position information.
        assert!(body.starts_with("[!] Job \"hello-gin\" - Failed at stage 3\n"));
    }

    #[test]
    fn failed_without_stop_reason_omits_reason_line() {
        // A bare `JobFailed` whose row has no `stop_reason` (the runtime
        // publishes before stamping in some legacy paths) still produces
        // a useful notification — the operator gets the stage header
        // and the cost, just without a reason word.
        let job = sample_job("orphan");
        let stage = sample_stage(0, "first", StageStatus::Failed);
        let body = format_job_failed(&job, Some(&stage), Some(1), None);
        assert!(body.contains("Failed at stage 1/1"));
        assert!(!body.contains("Reason:"));
    }

    #[test]
    fn stopped_renders_reason_from_event_payload() {
        let mut job = sample_job("scope-mutable-ui");
        // Deliberately leave the row's stop_reason empty so the test
        // exercises the "reason from event, not from row" path the
        // `JobStopped` renderer relies on.
        job.stop_reason = None;
        let stage = sample_stage(0, "first stage", StageStatus::Failed);
        let body = format_job_stopped(&job, Some(&stage), Some(4), StopReason::User, None);
        assert!(body.starts_with("[!] Job \"scope-mutable-ui\" - Stopped at stage 1/4"));
        assert!(body.contains("Reason: user-stopped"));
    }

    #[test]
    fn falls_back_when_template_name_unparseable() {
        let mut job = sample_job("ignored");
        job.template_yaml = None;
        let body = format_job_failed(&job, None, None, None);
        assert!(body.contains("(no-template)"), "got: {body}");
    }

    #[test]
    fn review_pre_check_auto_fail_renders_surface_2_block() {
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(7, "REVIEW after per-job action loop", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: Some(PreCheckOutcome::Fail {
                missing: vec![
                    "DOCS/SCOPE-MUTABLE-UI.md".to_string(),
                    "ui/codeless-ui/src/modules/jobs/patches".to_string(),
                ],
            }),
            verdict: Some(ReviewVerdict::AutoFail {
                reason: "diff-verify pre-check failed: handover claims paths not in the diff"
                    .to_string(),
            }),
        };
        let body = format_job_failed(&job, Some(&stage), Some(13), Some(&review));
        assert!(body.contains("Type:   diff-verify pre-check auto-fail"));
        assert!(body.contains("Missing paths:"));
        assert!(body.contains("      - DOCS/SCOPE-MUTABLE-UI.md"));
        assert!(body.contains("      - ui/codeless-ui/src/modules/jobs/patches"));
        assert!(body.contains("Verdict: diff-verify pre-check failed:"));
        // Surface 1 lines must still be present — the structured block
        // augments the notification rather than replacing it.
        assert!(body.contains("Stage:  \"REVIEW after per-job action loop\""));
        assert!(body.contains("Cost:   $52.64 / $150.00 cap"));
        assert!(body.contains("Reply in this thread: resume bypass | resume \"<comment>\" | stop"));
    }

    #[test]
    fn review_model_fail_renders_verdict_without_missing_paths() {
        // A `Fail` verdict without a prior pre-check fail is the model-
        // driven path. Missing-paths is absent (the model said FAIL but
        // the pre-check passed); the verdict text is the operator's
        // primary signal.
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(7, "REVIEW after sweep", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: Some(PreCheckOutcome::Pass {
                verified: vec!["src/x.rs".to_string()],
            }),
            verdict: Some(ReviewVerdict::Fail {
                reason: "handover bullet does not match the diff scope".to_string(),
            }),
        };
        let body = format_job_failed(&job, Some(&stage), Some(13), Some(&review));
        assert!(body.contains("Type:   model rejected handover"));
        assert!(!body.contains("Missing paths:"));
        assert!(body.contains("Verdict: handover bullet does not match the diff scope"));
    }

    #[test]
    fn review_auto_fail_without_pre_check_renders_runtime_auto_fail() {
        // Sentinel-missing / scope-patch-rejected paths fire a
        // ReviewVerdict::AutoFail without a prior ReviewPreCheck event.
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(7, "REVIEW after sweep", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: None,
            verdict: Some(ReviewVerdict::AutoFail {
                reason: "sentinel missing".to_string(),
            }),
        };
        let body = format_job_failed(&job, Some(&stage), Some(13), Some(&review));
        assert!(body.contains("Type:   runtime auto-fail"));
        assert!(!body.contains("Missing paths:"));
        assert!(body.contains("Verdict: sentinel missing"));
    }

    #[test]
    fn empty_review_context_collapses_to_surface_1() {
        // An empty (default) ReviewContext means the cache held no
        // entry for the failing stage — the renderer must NOT introduce
        // empty `Type:` / `Verdict:` lines, otherwise the Surface 2
        // block leaks visual noise into every non-REVIEW notification.
        let mut job = sample_job("hello-gin");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(0, "first stage", StageStatus::Failed);
        let body = format_job_failed(&job, Some(&stage), Some(3), Some(&ReviewContext::default()));
        assert!(!body.contains("Type:"));
        assert!(!body.contains("Missing paths:"));
        assert!(!body.contains("Verdict:"));
    }

    #[test]
    fn review_pre_check_fail_without_verdict_still_renders_paths() {
        // The pre-check `Fail` event arrives first; the matching
        // `ReviewVerdict::AutoFail` event might land after the
        // `JobFailed` envelope is dequeued. The renderer must still
        // surface the missing paths off the pre-check alone so the
        // notification is not blocked on a race.
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(7, "REVIEW", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: Some(PreCheckOutcome::Fail {
                missing: vec!["DOCS/SCOPE.md".to_string()],
            }),
            verdict: None,
        };
        let body = format_job_failed(&job, Some(&stage), Some(13), Some(&review));
        assert!(body.contains("Type:   diff-verify pre-check failed"));
        assert!(body.contains("Missing paths:"));
        assert!(body.contains("      - DOCS/SCOPE.md"));
        assert!(!body.contains("Verdict:"));
    }

    #[test]
    fn review_pre_check_pass_without_verdict_renders_no_block() {
        // A pre-check `Pass` with no verdict means the gate did not
        // fail — the JobFailed must be from somewhere else on the
        // stage. Showing a Surface 2 header for that case would
        // mislead the operator into looking at the gate.
        let mut job = sample_job("hello-gin");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(0, "REVIEW", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: Some(PreCheckOutcome::Pass {
                verified: vec!["x".to_string()],
            }),
            verdict: None,
        };
        let body = format_job_failed(&job, Some(&stage), Some(1), Some(&review));
        assert!(!body.contains("Type:"));
        assert!(!body.contains("Missing paths:"));
        assert!(!body.contains("Verdict:"));
    }

    #[test]
    fn review_block_renders_for_job_stopped_too() {
        // A REVIEW gate auto-fail that trips a cap-watcher stop (rare
        // but possible — a long-running auto-fail with a tight wall
        // clock) still surfaces the gate context on the JobStopped
        // notification because the operator's question — "do I
        // resume / bypass / stop" — is the same shape regardless of
        // the surrounding terminal verb.
        let mut job = sample_job("scope-mutable-ui");
        job.stop_reason = None;
        let stage = sample_stage(7, "REVIEW", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: None,
            verdict: Some(ReviewVerdict::AutoFail {
                reason: "scope-patch validation failed".to_string(),
            }),
        };
        let body = format_job_stopped(
            &job,
            Some(&stage),
            Some(13),
            StopReason::WallClock,
            Some(&review),
        );
        assert!(body.contains("Stopped at stage 8/13"));
        assert!(body.contains("Type:   runtime auto-fail"));
        assert!(body.contains("Verdict: scope-patch validation failed"));
        assert!(body.contains("Reason: wall-clock exceeded"));
    }

    #[test]
    fn review_block_skips_empty_missing_list() {
        // Defensive: a `Fail` whose missing vec is somehow empty (the
        // runtime should never emit this, but the wire shape allows
        // it) must not render an empty bullet list. The Type line
        // still goes out because the pre-check outcome itself is
        // informative.
        let mut job = sample_job("hello-gin");
        job.stop_reason = Some(StopReason::RunnerCrash);
        let stage = sample_stage(0, "REVIEW", StageStatus::Failed);
        let review = ReviewContext {
            pre_check: Some(PreCheckOutcome::Fail { missing: vec![] }),
            verdict: None,
        };
        let body = format_job_failed(&job, Some(&stage), Some(1), Some(&review));
        assert!(body.contains("Type:   diff-verify pre-check failed"));
        assert!(!body.contains("Missing paths:"));
    }

    #[test]
    fn is_empty_returns_true_only_when_both_fields_unset() {
        assert!(ReviewContext::default().is_empty());
        assert!(!ReviewContext {
            pre_check: Some(PreCheckOutcome::Skipped),
            verdict: None,
        }
        .is_empty());
        assert!(!ReviewContext {
            pre_check: None,
            verdict: Some(ReviewVerdict::Fail {
                reason: "x".to_string()
            }),
        }
        .is_empty());
    }
}
