//! Canned guidance comments threaded into the next stage's prompt
//! when an `AutoBypassPolicy` preset fires. Sourced verbatim from
//! `DOCS/AUTO-BYPASS-DECISIONS.md` Q4. Wording changes are a code
//! change (PR touches one line here + the matching line in the
//! decisions doc); meaning changes go through a fresh REVIEW gate
//! per Q4's wording-revision policy.
//!
//! Each string is a single paragraph, no leading/trailing newline.
//! The runtime wraps the resolved comment in an `Operator comment`
//! envelope above the stage goal — the same shape `resume_job`'s
//! `comment` argument uses today — so the next-stage runner reuses
//! the existing parser instead of growing a parallel one.

use codeless_types::AutoBypassPolicy;

pub const QUICK: &str = "Operator policy: Quick. The previous stage failed and auto-bypass advanced the job. Prefer the smallest change that produces a working result. Skip nice-to-haves; do not refactor surrounding code; do not add new abstractions.";

pub const LONG_TERM: &str = "Operator policy: Long-term. The previous stage failed and auto-bypass advanced the job. Prefer the durable fix over the quick one. Refactor for clarity if the next change would be harder without it. Tests are not optional; if you change behaviour, change the test that proves it.";

pub const CHEAP: &str = "Operator policy: Cheap. The previous stage failed and auto-bypass advanced the job. Minimise tokens and tool calls. Reuse existing helpers; do not write new infrastructure. If a one-line change unblocks the job, ship it and stop.";

pub const BEST_JUDGEMENT: &str = "Operator policy: Best judgement. The previous stage failed and auto-bypass advanced the job. The operator is not present to arbitrate quality versus speed. Use your own judgement on the trade-off for this stage; lean on the surrounding code and the project's CLAUDE.md rules to decide.";

pub const JUST_CODE: &str = "Operator policy: Just code. The previous stage failed and auto-bypass advanced the job. The operator wants forward progress. Pick a reasonable approach and ship it; do not block on questions, do not propose a SCOPE patch, do not request review unless the next change is destructive.";

/// Resolve a policy to the comment body the auto-bypass branch
/// threads into the next stage's prompt. Presets return their
/// `const &str` verbatim; `Custom` returns the operator's free-text
/// unchanged — the runtime never edits the contents (Q4).
pub fn policy_comment(policy: &AutoBypassPolicy) -> &str {
    match policy {
        AutoBypassPolicy::Quick => QUICK,
        AutoBypassPolicy::LongTerm => LONG_TERM,
        AutoBypassPolicy::Cheap => CHEAP,
        AutoBypassPolicy::BestJudgement => BEST_JUDGEMENT,
        AutoBypassPolicy::JustCode => JUST_CODE,
        AutoBypassPolicy::Custom { comment } => comment.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_comments_are_single_paragraph() {
        for s in [QUICK, LONG_TERM, CHEAP, BEST_JUDGEMENT, JUST_CODE] {
            assert!(!s.starts_with('\n'), "leading newline: {s:?}");
            assert!(!s.ends_with('\n'), "trailing newline: {s:?}");
            assert!(!s.contains("\n\n"), "paragraph break inside: {s:?}");
        }
    }

    #[test]
    fn custom_returns_operator_text_verbatim() {
        let policy = AutoBypassPolicy::Custom {
            comment: "do the thing".to_string(),
        };
        assert_eq!(policy_comment(&policy), "do the thing");
    }
}
