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
    Custom { comment: String },
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
        }
    }
}
