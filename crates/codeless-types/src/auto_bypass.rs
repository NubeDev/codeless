use serde::{Deserialize, Serialize};

/// Per-job auto-bypass policy. When set, a stage that fails under a
/// non-cap reason is marked `Failed`-with-`bypassed_at` automatically,
/// and the policy's canned (or operator-supplied) guidance is threaded
/// into the next stage's prompt instead of halting the job.
///
/// The five presets are reviewer-controlled canned guidance whose
/// exact comment strings live in
/// `codeless-runtime::auto_bypass_policy`. `Custom` carries the
/// operator's free-text comment verbatim — the runtime wraps it in the
/// same `Operator comment` envelope but does not edit the body.
///
/// Wire form is serde-`type`-tagged: presets serialize as
/// `{"type":"quick"}`, custom as
/// `{"type":"custom","comment":"..."}`. Cap-breach failures
/// (`StopReason::CostCap`, `StopReason::WallClock`) ignore the policy
/// and halt as today — operator-set caps win over auto-bypass per
/// `DOCS/AUTO-BYPASS-DECISIONS.md` Q2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AutoBypassPolicy {
    Quick,
    LongTerm,
    Cheap,
    BestJudgement,
    JustCode,
    Custom {
        comment: String,
    },
    /// Opt out of the two-strikes thrashing guard
    /// (`AUTO-BYPASS-DECISIONS.md` Q7). Intended for explicitly
    /// hands-off long-running jobs where the operator accepts that
    /// the only safety floor is the cost / wall-clock cap. The Q1
    /// guard still applies to every other policy variant — this is
    /// the one variant that disables it, and the disable is the
    /// whole point of the variant. Cap-breach failures continue to
    /// halt regardless (Q2 is not weakened).
    Relentless,
}

impl AutoBypassPolicy {
    /// Stable wire name used in events, audit lines, and the
    /// `policy_name` field on `StageAutoBypassed`. `Custom` resolves to
    /// the literal `"Custom"` — the operator's body is carried
    /// separately so the name stays a small finite enumeration the UI
    /// can render as a badge.
    pub fn policy_name(&self) -> &'static str {
        match self {
            AutoBypassPolicy::Quick => "Quick",
            AutoBypassPolicy::LongTerm => "Long-term",
            AutoBypassPolicy::Cheap => "Cheap",
            AutoBypassPolicy::BestJudgement => "Best-judgement",
            AutoBypassPolicy::JustCode => "Just code",
            AutoBypassPolicy::Custom { .. } => "Custom",
            AutoBypassPolicy::Relentless => "Relentless",
        }
    }

    /// Whether the two-strikes thrashing guard applies to this
    /// policy. `Relentless` is the only variant that opts out — see
    /// `AUTO-BYPASS-DECISIONS.md` Q7. Centralised here so the
    /// runtime's `classify_stage_failure` does not pattern-match on
    /// the variant directly and a future relaxation lands in one
    /// place.
    pub fn thrash_guard_applies(&self) -> bool {
        !matches!(self, AutoBypassPolicy::Relentless)
    }
}
