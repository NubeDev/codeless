//! Picker preset list for `AutoBypassPolicy`. The (id, label, hint)
//! tuple a UI surface — or the assistant planner — uses to enumerate
//! the operator-facing choices without re-stating the wording on each
//! call site.
//!
//! Hand-mirrored from `ui/codeless-ui/src/lib/policy/presets.ts`
//! (SCOPE-ASSISTANT-PARITY W3, open question 3). Seven variants do
//! not justify a code-generator; instead the parity test below reads
//! the TS file and asserts the two lists stay aligned. The variant
//! enum itself lives in `codeless_types::AutoBypassPolicy`; the
//! canned guidance strings the runtime threads into the next stage's
//! prompt live in `auto_bypass_policy`. This file owns only the
//! short label + one-line hint a picker / planner prompt renders
//! next to each id.
//!
//! `Custom` is intentionally absent — it has no fixed hint and a
//! picker renders it as a free-text field. The `None` sentinel is
//! also absent for the same reason: it represents the opt-out, not a
//! preset.
//!
//! Wording changes to a hint string travel as a paired edit: update
//! the entry here and the matching entry in the TS file in the same
//! commit. The parity test asserts the equality; CI fails loudly if
//! one side drifts.

use codeless_types::AutoBypassPolicy;

/// One preset row: the wire id (matching `AutoBypassPolicy`'s
/// serde-`type` tag for the variant), the short label a picker
/// renders, and a one-line hint used as the picker's helper text and
/// as the planner prompt's per-variant description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

/// The six preset variants of `AutoBypassPolicy`, in the order the
/// UI picker lists them. `Custom` is omitted — see module docs.
pub const POLICY_PRESETS: &[PolicyPreset] = &[
    PolicyPreset {
        id: "quick",
        label: "Quick",
        hint: "Smallest change that works; skip nice-to-haves and refactors.",
    },
    PolicyPreset {
        id: "long-term",
        label: "Long-term",
        hint: "Prefer the durable fix; tests stay in sync with behaviour.",
    },
    PolicyPreset {
        id: "cheap",
        label: "Cheap",
        hint: "Minimise tokens and tool calls; one-line fixes ship.",
    },
    PolicyPreset {
        id: "best-judgement",
        label: "Best judgement",
        hint: "Let the runner decide quality-vs-speed with no operator present.",
    },
    PolicyPreset {
        id: "just-code",
        label: "Just code",
        hint: "Pick a reasonable approach and ship it; do not block on questions.",
    },
    PolicyPreset {
        id: "relentless",
        label: "Relentless",
        hint: "Never stops on stage failure; only the cost cap and wall-clock cap halt the job.",
    },
];

/// Resolve a `PolicyPreset` row back to the matching
/// `AutoBypassPolicy` variant. Returns `None` for an unknown id; the
/// caller decides whether to reject or fall back to `None`-policy.
/// `Custom` is intentionally not reachable here — it carries
/// operator text the planner cannot synthesise from a preset row.
pub fn policy_for_preset_id(id: &str) -> Option<AutoBypassPolicy> {
    match id {
        "quick" => Some(AutoBypassPolicy::Quick),
        "long-term" => Some(AutoBypassPolicy::LongTerm),
        "cheap" => Some(AutoBypassPolicy::Cheap),
        "best-judgement" => Some(AutoBypassPolicy::BestJudgement),
        "just-code" => Some(AutoBypassPolicy::JustCode),
        "relentless" => Some(AutoBypassPolicy::Relentless),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_maps_to_a_variant() {
        for preset in POLICY_PRESETS {
            assert!(
                policy_for_preset_id(preset.id).is_some(),
                "preset id {:?} has no matching AutoBypassPolicy variant",
                preset.id,
            );
        }
    }

    /// Hand-mirror parity assert: the TS file's `POLICY_PRESETS`
    /// entries must match this Rust list byte-for-byte in id, label,
    /// and hint. Reading the TS file is enough — the wording lives in
    /// two places by design, and the test is the only thing keeping
    /// the two in lockstep (SCOPE-ASSISTANT-PARITY W3 open Q3).
    #[test]
    fn ts_mirror_in_sync() {
        let ts_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/codeless-ui/src/lib/policy/presets.ts");
        let src = std::fs::read_to_string(&ts_path)
            .unwrap_or_else(|err| panic!("read {:?}: {err}", ts_path));

        let mut found: Vec<(String, String, String)> = Vec::new();
        let mut current_id: Option<String> = None;
        let mut current_label: Option<String> = None;
        for line in src.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("id: \"") {
                let id = rest.trim_end_matches("\",").to_owned();
                current_id = Some(id);
                current_label = None;
            } else if let Some(rest) = line.strip_prefix("label: \"") {
                current_label = Some(rest.trim_end_matches("\",").to_owned());
            } else if let Some(rest) = line.strip_prefix("hint: \"") {
                let hint = rest.trim_end_matches("\",").to_owned();
                let id = current_id
                    .take()
                    .expect("hint without preceding id in presets.ts");
                let label = current_label
                    .take()
                    .expect("hint without preceding label in presets.ts");
                found.push((id, label, hint));
            }
        }

        let expected: Vec<(String, String, String)> = POLICY_PRESETS
            .iter()
            .map(|p| (p.id.to_owned(), p.label.to_owned(), p.hint.to_owned()))
            .collect();

        assert_eq!(
            found, expected,
            "POLICY_PRESETS drifted between the TS file and the Rust mirror; \
             update both in the same commit",
        );
    }
}
