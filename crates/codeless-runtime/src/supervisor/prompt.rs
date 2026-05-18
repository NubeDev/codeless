//! Supervisor system prompt + tool descriptions, kept as plain text so
//! a reviewer can read them in isolation without spelunking through
//! Rust. Three constraints shape the wording:
//!
//! 1. The supervisor's only outbound voice is `post_chat_message` (see
//!    `supervisor::mod` and JOB-CHAT.md §"Hard rules"). The prompt
//!    spells that out so a Claude-driven turn does not invent a
//!    sandbox command, an HTTP fetch, or any other escape hatch.
//! 2. Tools are read-only at this stage (C2 of JOB-CHAT.md). Action
//!    tools land in C3; the prompt deliberately does not mention them
//!    so a confused model does not hallucinate `stop_job` before the
//!    surface exists.
//! 3. The reply needs to stay short. The supervisor answers one
//!    question per chat turn and the typical reply fits in a tweet's
//!    worth of tokens; that keeps a per-turn cost below the
//!    JOB-CHAT.md (C2) §"Costs" target of <2c at current Anthropic
//!    pricing (Claude Haiku at ~$0.25/M input, ~$1.25/M output → the
//!    full system prompt + a small tool-result snippet + a one-paragraph
//!    reply lands well under that ceiling).
//!
//! The two `const` strings below are the load-bearing artifacts: a
//! later stage that swaps the reactor's hand-rolled matcher for a
//! Claude call composes `SYSTEM_PROMPT` + `\n\n` + `TOOL_DESCRIPTIONS`
//! as the `system_prompt` field on a `ClaudeRunnerAdapter`. They are
//! exercised by the unit tests at the bottom of this file (length
//! invariants + a structural smoke check that the tool descriptions
//! enumerate the seven tools the `Tools` struct exposes).

use codeless_types::{JobStatus, StageStatus};

/// Top-level role + voice contract. Short on purpose — the model
/// reads this every turn, and the tool descriptions below carry the
/// per-tool detail.
pub const SYSTEM_PROMPT: &str = "\
You are the per-Run supervisor inside Codeless. One Job is running on \
a worktree; you observe it through read-only tools and reply in a \
shared chat thread that the user reads from the web UI, Telegram, and \
any other connected transport. The user is not on a TTY — every reply \
you produce shows up as a chat message in that thread.\n\n\
Voice rules:\n\
- Your only output channel is the chat thread. Call `post_chat_message` \
exactly once per turn with your reply. Do not call it more than once; \
do not stay silent if the user asked a direct question.\n\
- Be specific. Cite stage names, ordinals, and the visible \
`failure_detail` when one is present. Never invent stage names, error \
strings, or session ids.\n\
- One short paragraph is the target. The user is reading this on a \
phone as often as not.\n\n\
Tool rules:\n\
- Tools are read-only. You cannot stop the job, edit files, or run \
shell commands.\n\
- Prefer `get_job_state` for \"what stage / how is it going\" \
questions; it summarises the row + stage list in one call.\n\
- Reach for `read_events`, `read_handover`, `read_stage_log`, or \
`read_notes` only when the user asked for detail that `get_job_state` \
does not carry. Each of those tools costs more than `get_job_state`.\n\
- If a tool returns NotFound or NotConfigured, say so plainly; do not \
guess what the missing data would have said.";

/// Per-tool documentation block. The shape mirrors what a tool-use
/// schema would carry — name, when to call, what it returns — so a
/// future stage can lift this text into a structured tool registry
/// without rewording.
pub const TOOL_DESCRIPTIONS: &str = "\
get_job_state(job_id) -> JobStateView\n\
    Snapshot of the running Job. Returns status, started_at, the \
    current stage (highest-ordinal Running stage, falling back to the \
    highest-ordinal stage at all), and total stage count. Use this \
    first for almost every question.\n\n\
read_events(job_id, limit) -> [EventEnvelope]\n\
    Tail of the persisted event stream for this Job. `limit` is \
    clamped to 500. Use for \"what just happened\" / \"why did stage \
    N take so long\" questions where the event timeline is the \
    answer.\n\n\
read_handover(job_id, stage_id) -> String\n\
    The `handover.md` the runner wrote at the end of a stage. \
    NotFound when the stage has not completed.\n\n\
read_template(job_id) -> Option<String>\n\
    The job's `template_yaml`, if it was submitted with one. Use to \
    answer \"what stages are left?\" without re-parsing the file.\n\n\
read_stage_log(job_id, stage_id) -> String\n\
    The per-stage activity log the recorder writes alongside the \
    handover.\n\n\
read_notes(job_id) -> [NoteFile]\n\
    Every file under `runs/<job_id>/notes/`, sorted by filename. \
    Empty when no notes have been written.\n\n\
post_chat_message(job_id, body)\n\
    The supervisor's only write tool. Inserts an assistant-role row \
    with `transport='supervisor'` and publishes the matching \
    `ChatMessageAppended` envelope so every transport sees the \
    reply. Call exactly once per turn.";

/// One stage row, projected to the fields the terminal summary
/// references. Borrowed because the caller already has `&Stage`
/// handles from the store and we do not want to clone the whole row
/// just to format a paragraph.
#[derive(Debug, Clone, Copy)]
pub struct TerminalStageInfo<'a> {
    pub ordinal: u32,
    pub name: &'a str,
    pub status: StageStatus,
    pub failure_detail: Option<&'a str>,
}

/// Compose the one-paragraph summary the supervisor posts to the chat
/// thread when the Run reaches a terminal status. The format is
/// deliberately mechanical — no LLM call — so the message is identical
/// across the `supervisor-claude` feature toggle and survives an
/// API-key outage. The shape:
///
/// > Run <status>. Stages: 1 stage-a (passed), 2 stage-b (failed: <detail>).
///
/// `failure_detail` is included verbatim when present (the underlying
/// column is already user-facing per JOB-MODEL.md). When a stage row
/// is missing a detail despite `status=Failed`, we elide the colon
/// rather than printing the literal "None".
pub fn format_terminal_summary(status: JobStatus, stages: &[TerminalStageInfo<'_>]) -> String {
    let status_label = match status {
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Stopped => "stopped",
        // The other variants are not terminal from `is_terminal_job`'s
        // perspective; the caller only invokes this fn on terminal
        // observation, so any other status here is a programming error
        // we surface in the text rather than panic on.
        other => return format!("Run reached non-terminal status {other:?}; no summary posted."),
    };
    if stages.is_empty() {
        return format!("Run {status_label}. No stages were recorded.");
    }
    let mut parts = Vec::with_capacity(stages.len());
    for s in stages {
        let stage_status = stage_status_label(s.status);
        let suffix = match (s.status, s.failure_detail) {
            (StageStatus::Failed, Some(detail)) if !detail.is_empty() => {
                format!(" ({stage_status}: {detail})")
            }
            _ => format!(" ({stage_status})"),
        };
        parts.push(format!("{} {}{}", s.ordinal, s.name, suffix));
    }
    format!("Run {}. Stages: {}.", status_label, parts.join(", "))
}

fn stage_status_label(s: StageStatus) -> &'static str {
    match s {
        StageStatus::Pending => "pending",
        StageStatus::Running => "running",
        StageStatus::AwaitingReview => "awaiting-review",
        StageStatus::Passed => "passed",
        StageStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost guard. Anthropic charges by token; for the Haiku price
    /// point in JOB-CHAT.md (C2) §"Costs", a system prompt over ~4 KB
    /// pushes a single-turn reply past the 2c-per-turn budget when
    /// combined with a small tool-result blob and a one-paragraph
    /// reply. The constant is loose — the actual budget shifts with
    /// price changes — but a regression that doubled the prompt size
    /// would trip this and force a fresh review.
    #[test]
    fn system_prompt_stays_small_enough_for_haiku_budget() {
        let combined = SYSTEM_PROMPT.len() + TOOL_DESCRIPTIONS.len();
        assert!(
            combined < 4096,
            "supervisor system prompt + tool descriptions must stay \
             under ~4 KB so a typical reply lands under the 2c-per-turn \
             Haiku budget; got {combined} bytes",
        );
    }

    /// Structural smoke: each of the seven tools on the `Tools` struct
    /// must be mentioned by name in `TOOL_DESCRIPTIONS`. A new tool
    /// added to `tools.rs` without a matching line here would let the
    /// model invent a signature.
    #[test]
    fn tool_descriptions_mention_every_supervisor_tool() {
        for name in [
            "get_job_state",
            "read_events",
            "read_handover",
            "read_template",
            "read_stage_log",
            "read_notes",
            "post_chat_message",
        ] {
            assert!(
                TOOL_DESCRIPTIONS.contains(name),
                "TOOL_DESCRIPTIONS must mention `{name}`",
            );
        }
    }

    #[test]
    fn terminal_summary_cites_stage_names_and_failure_detail() {
        let summary = format_terminal_summary(
            JobStatus::Failed,
            &[
                TerminalStageInfo {
                    ordinal: 1,
                    name: "bootstrap",
                    status: StageStatus::Passed,
                    failure_detail: None,
                },
                TerminalStageInfo {
                    ordinal: 2,
                    name: "build",
                    status: StageStatus::Failed,
                    failure_detail: Some("cargo: linker exit 1"),
                },
            ],
        );
        assert!(summary.contains("failed"));
        assert!(summary.contains("1 bootstrap"));
        assert!(summary.contains("2 build"));
        assert!(summary.contains("cargo: linker exit 1"));
    }

    #[test]
    fn terminal_summary_handles_completed_with_no_failure_details() {
        let summary = format_terminal_summary(
            JobStatus::Completed,
            &[TerminalStageInfo {
                ordinal: 1,
                name: "do-the-thing",
                status: StageStatus::Passed,
                failure_detail: None,
            }],
        );
        assert!(summary.starts_with("Run completed"));
        assert!(summary.contains("1 do-the-thing (passed)"));
        assert!(!summary.contains("None"));
    }

    #[test]
    fn terminal_summary_handles_no_stages() {
        let summary = format_terminal_summary(JobStatus::Stopped, &[]);
        assert!(summary.contains("stopped"));
        assert!(summary.contains("No stages"));
    }
}
