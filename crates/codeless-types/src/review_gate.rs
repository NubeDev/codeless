//! Wire types for the SESSION-MUTABLE-SCOPE Step 1 REVIEW-gate
//! diagnostics surface. The runtime already logs the Layer-1
//! diff-verify pre-check outcome and the model-driven sentinel
//! verdict via `tracing::info!` / `tracing::warn!`; these structs
//! carry the same information on the event bus so the editor's
//! `ReviewGatePanel` (Surface A) can render gate-failure context
//! without scraping logs.
//!
//! Lives in `codeless-types` (mobile-safe) for the same reason
//! `scope_patch.rs` does: the mobile shell builds `-types` plus
//! `-client` only and must decode every event it receives. The
//! `tracing` calls in `template_runner.rs` stay — these types
//! ride alongside them, they do not replace them.

use serde::{Deserialize, Serialize};

/// Result of the Layer-1 diff-verify pre-check the runtime runs
/// against the prior stage's handover before invoking the REVIEW
/// model. `Pass` and `Fail` carry the path lists the verifier
/// resolved so the UI can render the exact set rather than a
/// boolean glyph; `Skipped` and `NothingToVerify` are split so the
/// UI can distinguish "no prior handover / git enumeration failed"
/// from "handover present but no path-shaped tokens to verify"
/// (the former is a setup gap, the latter is a clean baseline).
///
/// The internal `template_runner::PreCheckOutcome` carries a single
/// `Fail(String)` and collapses the verified-path list; this wire
/// variant promotes both halves so the panel renders the actual
/// paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum PreCheckOutcome {
    /// Every path the prior handover's `Done` section claimed
    /// resolved to a real diff entry. `verified` is the claimed
    /// path text, in the order the handover listed it.
    Pass { verified: Vec<String> },
    /// At least one claimed path was absent from the diff.
    /// `missing` is the claimed-path text for each missing entry,
    /// in the order the handover listed them. The runtime's
    /// `diff_verify::MissingPath` also carries fuzzy candidates
    /// for each miss; this wire shape keeps only the claim text
    /// because the panel surfaces the miss list directly and the
    /// candidates were never reported in the existing log line.
    Fail { missing: Vec<String> },
    /// The pre-check could not run: no prior stage on the job, no
    /// worktree on the run context, the prior handover file was
    /// absent, git enumeration failed, or the spawn_blocking task
    /// joined with an error. The runtime treats this identically
    /// to `Pass` (the inner adapter still runs); the wire variant
    /// is kept distinct so the panel renders a setup-gap glyph
    /// rather than a green check.
    Skipped,
    /// The pre-check ran and the prior handover's `Done` named no
    /// path-shaped tokens. Distinct from `Skipped` because the
    /// handover was readable and the diff was enumerable; there
    /// was simply nothing the verifier could match against.
    NothingToVerify,
}

/// REVIEW-stage verdict surfaced on the event bus. The runtime
/// keeps its own `ReviewVerdict` next to the sentinel parser; this
/// wire variant adds `AutoFail` for the cases where the model was
/// never invoked (pre-check rejected the stage) or where the
/// post-stage validator rejected the gate before the verdict could
/// be acted on (sentinel unparseable, scope-patch validation
/// failed). The reason text mirrors what the matching `tracing`
/// call already records, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "verdict", rename_all = "kebab-case")]
pub enum ReviewVerdict {
    /// The model wrote a `PASS:` sentinel and downstream patch
    /// validation accepted the handover.
    Pass { reason: String },
    /// The model wrote a `FAIL:` sentinel.
    Fail { reason: String },
    /// The runtime closed the gate without (or in spite of) the
    /// model's verdict: pre-check fail, sentinel missing /
    /// ambiguous, or scope-patch validation rejected.
    AutoFail { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_check_outcome_pass_serialises_with_outcome_tag() {
        let v = serde_json::to_value(PreCheckOutcome::Pass {
            verified: vec!["a/b.rs".into()],
        })
        .unwrap();
        assert_eq!(v["outcome"], "pass");
        assert_eq!(v["verified"], serde_json::json!(["a/b.rs"]));
    }

    #[test]
    fn pre_check_outcome_unit_variants_serialise_as_kebab_case() {
        let v = serde_json::to_value(PreCheckOutcome::Skipped).unwrap();
        assert_eq!(v["outcome"], "skipped");
        let v = serde_json::to_value(PreCheckOutcome::NothingToVerify).unwrap();
        assert_eq!(v["outcome"], "nothing-to-verify");
    }

    #[test]
    fn review_verdict_auto_fail_serialises_with_verdict_tag() {
        let v = serde_json::to_value(ReviewVerdict::AutoFail {
            reason: "diff-verify pre-check failed".into(),
        })
        .unwrap();
        assert_eq!(v["verdict"], "auto-fail");
        assert_eq!(v["reason"], "diff-verify pre-check failed");
    }

    #[test]
    fn pre_check_outcome_roundtrips() {
        let cases = [
            PreCheckOutcome::Pass {
                verified: vec!["x".into(), "y".into()],
            },
            PreCheckOutcome::Fail {
                missing: vec!["z".into()],
            },
            PreCheckOutcome::Skipped,
            PreCheckOutcome::NothingToVerify,
        ];
        for c in cases {
            let v = serde_json::to_value(&c).unwrap();
            let back: PreCheckOutcome = serde_json::from_value(v).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn review_verdict_roundtrips() {
        let cases = [
            ReviewVerdict::Pass {
                reason: "looks good".into(),
            },
            ReviewVerdict::Fail {
                reason: "missing tests".into(),
            },
            ReviewVerdict::AutoFail {
                reason: "sentinel missing".into(),
            },
        ];
        for c in cases {
            let v = serde_json::to_value(&c).unwrap();
            let back: ReviewVerdict = serde_json::from_value(v).unwrap();
            assert_eq!(back, c);
        }
    }
}
