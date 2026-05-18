//! Wire types for scoped pause points — operator-declared breakpoints
//! that the runtime trips on its own instead of waiting for a runtime
//! pause click. The grammar these shapes back lives in
//! `DOCS/SCOPED-PAUSE-POINTS.md`; this module is the wire surface the
//! parser (stage 4), the persistence layer (stage 5), and the runtime
//! hook (stage 6) all marshal through.
//!
//! Lives in `codeless-types` so the mobile shell — which builds
//! `-types` + `-client` only — sees the same shape the host emits on
//! the bus, per R1. A scoped pause is *scheduling on top of an
//! existing primitive*: the runtime still calls `pause_job` and emits
//! the existing `JobPaused` event when one of these trips, so no
//! state-machine surface lives here.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::todo::TodoKind;

/// Identity of one scheduled pause point. Defined inline rather than
/// added to the shared `ulid_newtype!` macro in `id.rs` to keep the
/// pause-point types one self-contained unit — same rationale
/// `ScopePatchId` calls out in `scope_patch.rs`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(transparent)]
#[specta(transparent)]
pub struct PausePointId(#[specta(type = String)] pub Ulid);

impl PausePointId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for PausePointId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PausePointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for PausePointId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(s).map(Self)
    }
}

/// Which side of the targeted boundary the pause fires on. `position:`
/// is a required YAML key — `pause stage 3` is ambiguous between "halt
/// before the runner spawns" and "halt after the trio closes", and
/// forcing the keyword removes the foot-gun (see SCOPED-PAUSE-POINTS
/// §1.3 and §5 Q1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum PausePointPosition {
    Before,
    After,
}

/// How a todo inside a stage is named. `Ordinal` is a 1-based index
/// into the stage's todo list at trigger time; `Trio` names one of the
/// runtime-injected closing-trio kinds (always resolvable at submit
/// time); `TitleSubstring` is a case-insensitive `contains` match
/// evaluated at trigger time so runner-authored todos that don't exist
/// at parse time can still be addressed.
///
/// Trio kinds are reserved words at the YAML layer: `todo: checks`
/// always resolves to `Trio(Checks)`, even if a runner-authored todo
/// happens to share the title. To target a non-trio todo whose title
/// contains a trio word, use `~checks` (the substring form) — see
/// SCOPED-PAUSE-POINTS §1.2.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "selector")]
pub enum TodoSelector {
    #[serde(rename = "ordinal")]
    Ordinal { ordinal: u32 },
    #[serde(rename = "trio")]
    Trio { kind: TodoKind },
    #[serde(rename = "title-substring")]
    TitleSubstring { pattern: String },
}

/// What the pause point fires against. `Stage` targets the stage
/// boundary itself (provisioning side on `Before`, post-trio side on
/// `After`); `StageTodo` narrows the trigger to one todo inside the
/// stage. The selector lives on `StageTodo` rather than as a third
/// `Stage`-variant field so the absence of a selector is a type-level
/// fact, not a `None` to forget to handle.
///
/// `stage_ordinal` is always the resolved 1-based index into the
/// template's `stages:` list. Name-form targets (`stage: parser`) are
/// resolved to an ordinal by the parser before a row reaches this
/// type, so the wire shape and the SQLite shape match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind")]
pub enum PausePointTarget {
    #[serde(rename = "stage")]
    Stage {
        ordinal: u32,
    },
    #[serde(rename = "stage-todo")]
    StageTodo {
        stage_ordinal: u32,
        selector: TodoSelector,
    },
}

/// One scheduled pause point as it travels on the wire. The parser
/// (stage 4) builds one of these per `pause_points:` entry; the
/// persistence layer (stage 5) writes it into `scheduled_pause_points`
/// keyed on `(job_id, ordinal)`; the runtime hook (stage 6) reads it
/// back when checking whether to trip `pause_job`.
///
/// `reason` is the operator's free-text justification, surfaced
/// verbatim in the chat divider label and in the `StopReason::
/// ScopedPausePoint` payload. The 512-byte cap is enforced by the
/// parser, not by this struct — keeping the cap a parse-time rule
/// lets future iterations relax it without a wire migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PausePoint {
    pub id: PausePointId,
    pub target: PausePointTarget,
    pub position: PausePointPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn pause_point_id_roundtrips_through_string() {
        let id = PausePointId::new();
        let s = id.to_string();
        let back = PausePointId::from_str(&s).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn position_serialises_kebab_case() {
        assert_eq!(
            serde_json::to_string(&PausePointPosition::Before).unwrap(),
            "\"before\""
        );
        assert_eq!(
            serde_json::to_string(&PausePointPosition::After).unwrap(),
            "\"after\""
        );
    }

    #[test]
    fn todo_selector_serialises_with_selector_tag() {
        let ord = TodoSelector::Ordinal { ordinal: 2 };
        assert_eq!(
            serde_json::to_string(&ord).unwrap(),
            r#"{"selector":"ordinal","ordinal":2}"#
        );
        let trio = TodoSelector::Trio {
            kind: TodoKind::Docs,
        };
        assert_eq!(
            serde_json::to_string(&trio).unwrap(),
            r#"{"selector":"trio","kind":"docs"}"#
        );
        let sub = TodoSelector::TitleSubstring {
            pattern: "migrate".into(),
        };
        assert_eq!(
            serde_json::to_string(&sub).unwrap(),
            r#"{"selector":"title-substring","pattern":"migrate"}"#
        );
    }

    #[test]
    fn todo_selector_variants_roundtrip_independently() {
        for s in [
            TodoSelector::Ordinal { ordinal: 7 },
            TodoSelector::Trio {
                kind: TodoKind::Checks,
            },
            TodoSelector::Trio {
                kind: TodoKind::Git,
            },
            TodoSelector::TitleSubstring {
                pattern: "schema".into(),
            },
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: TodoSelector = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn stage_target_roundtrips_through_json() {
        let p = PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::Stage { ordinal: 3 },
            position: PausePointPosition::Before,
            reason: Some("spot-check wire types".into()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PausePoint = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn stage_todo_trio_target_roundtrips_through_json() {
        let p = PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: 5,
                selector: TodoSelector::Trio {
                    kind: TodoKind::Docs,
                },
            },
            position: PausePointPosition::After,
            reason: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PausePoint = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn stage_todo_substring_target_roundtrips_through_json() {
        let p = PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: 3,
                selector: TodoSelector::TitleSubstring {
                    pattern: "migrate".into(),
                },
            },
            position: PausePointPosition::Before,
            reason: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PausePoint = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn stage_todo_ordinal_target_roundtrips_through_json() {
        let p = PausePoint {
            id: PausePointId::new(),
            target: PausePointTarget::StageTodo {
                stage_ordinal: 2,
                selector: TodoSelector::Ordinal { ordinal: 4 },
            },
            position: PausePointPosition::After,
            reason: Some("between steps".into()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PausePoint = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn missing_reason_field_deserialises_as_none() {
        let json = r#"{"id":"00000000000000000000000000","target":{"kind":"stage","ordinal":1},"position":"before"}"#;
        let p: PausePoint = serde_json::from_str(json).unwrap();
        assert_eq!(p.reason, None);
    }
}
