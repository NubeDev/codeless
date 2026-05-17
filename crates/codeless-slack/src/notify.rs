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
//! The mockup in the SCOPE doc uses an emoji glyph (`U+1F6A8`); the
//! repo-wide rule (CLAUDE.md R2) forbids them, so the renderer uses
//! the same `[!]` tag the inbound-reply path settled on for warnings
//! / alerts. The tag carries the same meaning — "something needs
//! operator attention" — so a Slack reader who already knows the
//! reply-path conventions reads the outbound shape the same way.
//!
//! Renderers stay pure: they take the inputs already resolved by
//! the publisher (job row, optional stage rollup with the ordinal
//! and title) and produce a string. Network access and event-bus
//! filtering live next door in [`crate::outbound`]; keeping the
//! split lets the formatting be unit-tested without spawning a
//! Slack stub.

use codeless_rpc::methods::StageRollup;
use codeless_types::{Job, StopReason};

use crate::reply::template_name;

/// Format a `JobFailed` notification. `stage` is the failing stage
/// rollup (`None` when the publisher's `list_stages` call returned
/// nothing — the notification still goes out, just without the
/// stage-N/N header). `total_stages` is the count of stages on the
/// job (also `None` when the call failed); the header degrades to
/// `"stage 8"` when only one of the two is available, and to a bare
/// `"Failed"` when neither is.
pub fn format_job_failed(
    job: &Job,
    stage: Option<&StageRollup>,
    total_stages: Option<u32>,
) -> String {
    render(job, stage, total_stages, OutboundEvent::Failed)
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
) -> String {
    render(job, stage, total_stages, OutboundEvent::Stopped(reason))
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
        let body = format_job_failed(&job, Some(&stage), Some(13));
        assert!(body.starts_with("[!] Job \"scope-mutable-ui\" - Failed at stage 8/13"));
        assert!(body.contains("Stage:  \"REVIEW after per-job action loop\""));
        assert!(body.contains("Reason: runner crashed"));
        assert!(body.contains("Cost:   $52.64 / $150.00 cap"));
        assert!(
            body.contains("Reply in this thread: resume bypass | resume \"<comment>\" | stop"),
            "missing reply hint in:\n{body}",
        );
    }

    #[test]
    fn failed_without_stage_rollup_degrades_to_bare_header() {
        let mut job = sample_job("smscope-smoke");
        job.stop_reason = Some(StopReason::CostCap);
        let body = format_job_failed(&job, None, None);
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
        let body = format_job_failed(&job, Some(&stage), None);
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
        let body = format_job_failed(&job, Some(&stage), Some(1));
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
        let body = format_job_stopped(&job, Some(&stage), Some(4), StopReason::User);
        assert!(body.starts_with("[!] Job \"scope-mutable-ui\" - Stopped at stage 1/4"));
        assert!(body.contains("Reason: user-stopped"));
    }

    #[test]
    fn falls_back_when_template_name_unparseable() {
        let mut job = sample_job("ignored");
        job.template_yaml = None;
        let body = format_job_failed(&job, None, None);
        assert!(body.contains("(no-template)"), "got: {body}");
    }
}
