//! Canned guidance comments threaded into the next stage's prompt
//! when an `AutoBypassPolicy` preset fires. Sourced verbatim from
//! `DOCS/AUTO-BYPASS-DECISIONS.md` Q4. Wording changes are a code
//! change (PR touches one line here + the matching line in the
//! decisions doc); meaning changes go through a fresh REVIEW gate
//! per Q4's wording-revision policy.
//!
//! Each preset string is a single paragraph, no leading/trailing
//! newline. The runtime wraps the resolved comment in an `Operator
//! comment` envelope above the stage goal — the same shape
//! `resume_job`'s `comment` argument uses today — so the next-stage
//! runner reuses the existing parser instead of growing a parallel
//! one.
//!
//! When the prior stage carries a `failure_class`, `policy_comment`
//! appends a fenced `Previous-stage failure:` block after the policy
//! paragraph so the next-stage prompt names the concrete reason
//! auto-bypass fired. Shape, ordering, and truncation rules are
//! pinned in `SCOPE.md` Q4 of the auto-bypass-hardening job; the
//! tests below are the executable copy of that contract.

use codeless_types::{AutoBypassPolicy, FailureClass};

pub const QUICK: &str = "Operator policy: Quick. The previous stage failed and auto-bypass advanced the job. Prefer the smallest change that produces a working result. Skip nice-to-haves; do not refactor surrounding code; do not add new abstractions.";

pub const LONG_TERM: &str = "Operator policy: Long-term. The previous stage failed and auto-bypass advanced the job. Prefer the durable fix over the quick one. Refactor for clarity if the next change would be harder without it. Tests are not optional; if you change behaviour, change the test that proves it.";

pub const CHEAP: &str = "Operator policy: Cheap. The previous stage failed and auto-bypass advanced the job. Minimise tokens and tool calls. Reuse existing helpers; do not write new infrastructure. If a one-line change unblocks the job, ship it and stop.";

pub const BEST_JUDGEMENT: &str = "Operator policy: Best judgement. The previous stage failed and auto-bypass advanced the job. The operator is not present to arbitrate quality versus speed. Use your own judgement on the trade-off for this stage; lean on the surrounding code and the project's CLAUDE.md rules to decide.";

pub const JUST_CODE: &str = "Operator policy: Just code. The previous stage failed and auto-bypass advanced the job. The operator wants forward progress. Pick a reasonable approach and ship it; do not block on questions, do not propose a SCOPE patch, do not request review unless the next change is destructive.";

pub const RELENTLESS: &str = "Operator policy: Relentless. The previous stage failed and auto-bypass advanced the job. The thrashing guard is disabled for this job; the only stop signals are the cost cap and the wall-clock cap. Make the best long-term decision you can with the context you have, ship forward progress, and keep going — the operator is not present to arbitrate.";

/// Maximum number of Unicode scalar values from the prior stage's
/// `failure_detail` that the threaded prompt carries. The stored
/// row value is unchanged; this is the prompt-boundary ceiling that
/// keeps the per-stage prefix bounded even if a future failure
/// path captures a longer detail than the recorder's own ceiling.
const PROMPT_DETAIL_MAX_CHARS: usize = 400;

/// Single character ellipsis (U+2026) — one char so the truncation
/// boundary math does not have to budget for a three-char marker.
const TRUNCATION_MARKER: char = '…';

/// Resolve a preset to its canonical canned paragraph. `Custom`
/// returns the operator's free-text unchanged. The runtime never
/// edits the contents — that is Q4's wording-revision policy.
fn policy_paragraph(policy: &AutoBypassPolicy) -> &str {
    match policy {
        AutoBypassPolicy::Quick => QUICK,
        AutoBypassPolicy::LongTerm => LONG_TERM,
        AutoBypassPolicy::Cheap => CHEAP,
        AutoBypassPolicy::BestJudgement => BEST_JUDGEMENT,
        AutoBypassPolicy::JustCode => JUST_CODE,
        AutoBypassPolicy::Custom { comment } => comment.as_str(),
        AutoBypassPolicy::Relentless => RELENTLESS,
    }
}

/// Prior-stage failure context threaded into the next stage's
/// prompt when auto-bypass advances the job. Carried by value
/// because the stage row is loaded outside the runtime crate and
/// the borrow would survive the row drop.
#[derive(Debug, Clone)]
pub struct PriorFailure {
    pub class: FailureClass,
    pub detail: String,
}

/// Kebab-case wire name of a `FailureClass` — the same string the
/// events stream carries, so grep-the-log forensics match what the
/// model sees in its prompt.
fn failure_class_wire_name(class: FailureClass) -> &'static str {
    match class {
        FailureClass::PreCheckFailed => "pre-check-failed",
        FailureClass::RunnerError => "runner-error",
        FailureClass::InfrastructureError => "infrastructure-error",
        FailureClass::ReviewPatchInvalid => "review-patch-invalid",
        FailureClass::ReviewFail => "review-fail",
        FailureClass::ReviewUnparseable => "review-unparseable",
        FailureClass::OrphanReap => "orphan-reap",
    }
}

/// Trim trailing whitespace + newlines and cap the result at
/// `PROMPT_DETAIL_MAX_CHARS` Unicode scalar values, appending the
/// single-char ellipsis when truncation fires. The boundary count
/// uses `chars()` so a multibyte run never splits mid-codepoint.
/// Returns `None` when the normalised detail is empty so the caller
/// can omit the `Detail:` line entirely (the `OrphanReap` path and
/// any future class that does not always carry a detail).
fn normalise_detail(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches(|c: char| c.is_whitespace());
    if trimmed.is_empty() {
        return None;
    }
    let count = trimmed.chars().count();
    if count <= PROMPT_DETAIL_MAX_CHARS {
        return Some(trimmed.to_string());
    }
    let mut out: String = trimmed.chars().take(PROMPT_DETAIL_MAX_CHARS).collect();
    out.push(TRUNCATION_MARKER);
    Some(out)
}

/// Assemble the operator-comment string threaded into the next
/// stage's prompt when auto-bypass advances the job.
///
/// `prior == None` reproduces the pre-stage-7 behaviour byte-for-
/// byte: the canned policy paragraph (or `Custom` free-text) is
/// returned with no fenced block, no trailing blank line, and no
/// other decoration. That keeps the no-prior-failure path — stage 0
/// of a job, or a `Passed` previous stage — identical to the wire
/// shape the existing integration tests pin.
///
/// `prior == Some(_)` appends a blank line followed by a fenced
/// block of the form
///
/// ```text
/// Previous-stage failure: <wire-name>
/// Detail: <failure_detail truncated to PROMPT_DETAIL_MAX_CHARS>
/// ```
///
/// with the `Detail:` line omitted when the normalised detail is
/// empty (e.g. `OrphanReap` rows whose detail is often blank).
/// The fence is a triple-backtick fence with no language tag so it
/// renders as plain preformatted text in every chat UI and is not
/// mistakenly highlighted as code.
pub fn policy_comment(policy: &AutoBypassPolicy, prior: Option<&PriorFailure>) -> String {
    let head = policy_paragraph(policy);
    let Some(prior) = prior else {
        return head.to_string();
    };
    let wire_name = failure_class_wire_name(prior.class);
    let detail = normalise_detail(&prior.detail);
    let mut out = String::with_capacity(head.len() + 64 + detail.as_deref().map_or(0, str::len));
    out.push_str(head);
    out.push_str("\n\n```\n");
    out.push_str("Previous-stage failure: ");
    out.push_str(wire_name);
    if let Some(detail) = detail {
        out.push_str("\nDetail: ");
        out.push_str(&detail);
    }
    out.push_str("\n```");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_comments_are_single_paragraph() {
        for s in [
            QUICK,
            LONG_TERM,
            CHEAP,
            BEST_JUDGEMENT,
            JUST_CODE,
            RELENTLESS,
        ] {
            assert!(!s.starts_with('\n'), "leading newline: {s:?}");
            assert!(!s.ends_with('\n'), "trailing newline: {s:?}");
            assert!(!s.contains("\n\n"), "paragraph break inside: {s:?}");
        }
    }

    #[test]
    fn custom_returns_operator_text_verbatim_when_no_prior() {
        let policy = AutoBypassPolicy::Custom {
            comment: "do the thing".to_string(),
        };
        assert_eq!(policy_comment(&policy, None), "do the thing");
    }

    #[test]
    fn no_prior_returns_bare_policy_text() {
        assert_eq!(policy_comment(&AutoBypassPolicy::Quick, None), QUICK);
        assert_eq!(policy_comment(&AutoBypassPolicy::LongTerm, None), LONG_TERM);
    }

    #[test]
    fn pre_check_failed_with_detail_threads_fenced_block() {
        let prior = PriorFailure {
            class: FailureClass::PreCheckFailed,
            detail: "diff touches src/foo.rs but Done lists src/bar.rs".to_string(),
        };
        let got = policy_comment(&AutoBypassPolicy::Quick, Some(&prior));
        let want = format!(
            "{QUICK}\n\n```\nPrevious-stage failure: pre-check-failed\nDetail: diff touches src/foo.rs but Done lists src/bar.rs\n```"
        );
        assert_eq!(got, want);
    }

    #[test]
    fn review_fail_with_detail_threads_fenced_block() {
        let prior = PriorFailure {
            class: FailureClass::ReviewFail,
            detail: "acceptance bullet 2 not satisfied".to_string(),
        };
        let got = policy_comment(&AutoBypassPolicy::LongTerm, Some(&prior));
        let want = format!(
            "{LONG_TERM}\n\n```\nPrevious-stage failure: review-fail\nDetail: acceptance bullet 2 not satisfied\n```"
        );
        assert_eq!(got, want);
    }

    #[test]
    fn none_failure_class_path_is_bare_policy_text() {
        // `None` is the prior-stage shape for stage 0 of a job and
        // for a `Passed` previous stage. Pinning it here keeps the
        // two integration tests that match the bare canned strings
        // passing byte-for-byte after the signature widened.
        let got = policy_comment(&AutoBypassPolicy::Cheap, None);
        assert_eq!(got, CHEAP);
    }

    #[test]
    fn empty_detail_omits_detail_line() {
        let prior = PriorFailure {
            class: FailureClass::OrphanReap,
            detail: String::new(),
        };
        let got = policy_comment(&AutoBypassPolicy::BestJudgement, Some(&prior));
        let want = format!("{BEST_JUDGEMENT}\n\n```\nPrevious-stage failure: orphan-reap\n```");
        assert_eq!(got, want);
    }

    #[test]
    fn whitespace_only_detail_omits_detail_line() {
        let prior = PriorFailure {
            class: FailureClass::OrphanReap,
            detail: "   \n\t\n".to_string(),
        };
        let got = policy_comment(&AutoBypassPolicy::BestJudgement, Some(&prior));
        let want = format!("{BEST_JUDGEMENT}\n\n```\nPrevious-stage failure: orphan-reap\n```");
        assert_eq!(got, want);
    }

    #[test]
    fn detail_trailing_whitespace_is_stripped() {
        let prior = PriorFailure {
            class: FailureClass::RunnerError,
            detail: "exit 1: panicked at lib.rs:42\n\n".to_string(),
        };
        let got = policy_comment(&AutoBypassPolicy::Quick, Some(&prior));
        assert!(
            got.ends_with("Detail: exit 1: panicked at lib.rs:42\n```"),
            "trailing whitespace not stripped: {got:?}"
        );
    }

    #[test]
    fn long_detail_is_truncated_with_single_char_ellipsis() {
        let detail = "x".repeat(500);
        let prior = PriorFailure {
            class: FailureClass::RunnerError,
            detail,
        };
        let got = policy_comment(&AutoBypassPolicy::Quick, Some(&prior));
        let detail_line = got
            .lines()
            .find(|l| l.starts_with("Detail: "))
            .expect("Detail line present");
        let payload = &detail_line["Detail: ".len()..];
        assert_eq!(
            payload.chars().count(),
            PROMPT_DETAIL_MAX_CHARS + 1,
            "payload should be max + 1 ellipsis"
        );
        assert!(
            payload.ends_with(TRUNCATION_MARKER),
            "truncated payload must end with the single-char ellipsis"
        );
    }

    #[test]
    fn detail_exactly_at_limit_is_not_truncated() {
        let detail = "y".repeat(PROMPT_DETAIL_MAX_CHARS);
        let prior = PriorFailure {
            class: FailureClass::RunnerError,
            detail: detail.clone(),
        };
        let got = policy_comment(&AutoBypassPolicy::Quick, Some(&prior));
        assert!(
            got.contains(&format!("Detail: {detail}\n```")),
            "detail at exact ceiling must not be truncated"
        );
        assert!(
            !got.contains(TRUNCATION_MARKER),
            "no truncation marker when at exact ceiling"
        );
    }

    #[test]
    fn wire_names_match_serde_kebab_case() {
        // Pin the wire-name table against `serde_json` so a future
        // rename of a `FailureClass` variant trips this test before
        // the prompt-side spelling drifts from the events stream.
        for class in [
            FailureClass::PreCheckFailed,
            FailureClass::RunnerError,
            FailureClass::InfrastructureError,
            FailureClass::ReviewPatchInvalid,
            FailureClass::ReviewFail,
            FailureClass::ReviewUnparseable,
            FailureClass::OrphanReap,
        ] {
            let json = serde_json::to_value(class).expect("serialise FailureClass");
            let wire = json.as_str().expect("FailureClass serialises to a string");
            assert_eq!(
                failure_class_wire_name(class),
                wire,
                "wire name drifted from serde rename for {class:?}",
            );
        }
    }
}
