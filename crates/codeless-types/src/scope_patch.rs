//! Wire types for the SESSION-MUTABLE-SCOPE ramp's patch proposals.
//!
//! Lives in `codeless-types` (not `-runtime`) so the mobile shell —
//! which builds `-types` + `-client` only — sees the same shapes the
//! host emits on the event bus. The decision is recorded in
//! `DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md` Q7: a patch proposal is
//! an event on the existing bus, not a row in a new SQLite table, and
//! the discriminants travel over SSE as soon as Step 4 lands.
//!
//! Step 4 ships the *shadow-mode* shape: the proposals accumulate in
//! `DOCS/SCOPE-PROPOSED.md` and the runtime emits `ScopePatchProposed`
//! envelopes, but nothing merges automatically. Step 5 layers the
//! parse-time guards (one-per-REVIEW, mutable-set membership, evidence
//! requirements); the field set here is the floor those guards check
//! against, so no fields move out of this struct in Step 5 — only
//! validation tightens.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::id::{ReviewId, StageId};

/// Identity of one `ScopePatch` proposal. ULID for the same reason
/// every other identity in `codeless-types::id` is a ULID: monotonic
/// over a session, sortable by creation order, and unambiguous on the
/// wire. Defined here rather than in `id.rs` because adding it to the
/// shared `ulid_newtype!` macro would force every consumer (mobile
/// included) to depend on the macro the moment a `ScopePatchId`
/// imports — keeping it in this module keeps the patch types one
/// logical unit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(transparent)]
#[specta(transparent)]
pub struct ScopePatchId(#[specta(type = String)] pub Ulid);

impl ScopePatchId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for ScopePatchId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ScopePatchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ScopePatchId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(s).map(Self)
    }
}

/// Whether a patch makes a rule stricter (`Tighten`) or weaker
/// (`Loosen`). Per `SESSION-MUTABLE-SCOPE-DECISIONS.md` Q2, rule
/// *removal* is `Loosen`: after the patch lands the rule no longer
/// constrains, which is observably the same effect as a
/// textually-narrower replacement. Predicate-file deletion is **not**
/// a third kind — it rides on a paired `Loosen` patch in the
/// approving human's commit (decisions Q5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ScopePatchKind {
    Tighten,
    Loosen,
}

/// Which rulebook surface the patch proposes to change. The enum
/// names the *category* the surface belongs to; the exact file path
/// travels separately on the `ScopePatch.target_path` field so the
/// human approval UX in Step 6 can show "this proposal edits
/// `.codeless/jobs/foo/SCOPE.md`" without needing a per-job variant
/// here.
///
/// The mutable-set vs wire-format-set distinction in
/// `codeless-runtime::rule_bearing_files` is enforced at parse time
/// in Step 5; this enum lists only categories that are members of the
/// mutable set. Wire-format files (`DOCS/JOB-MODEL.md`,
/// `DOCS/JOB-LOOP.md`, `codeless-types/src/handover.rs`) have no
/// variant here on purpose — proposing a patch against one is a
/// parse-time reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ScopePatchTarget {
    /// Workspace-level `CLAUDE.md` or the inner repo's `CLAUDE.md`.
    ClaudeMd,
    /// A per-job `SCOPE.md` under `.codeless/jobs/<name>/`.
    JobScopeMd,
    /// A per-job `WORKFLOW.md` under `.codeless/jobs/<name>/`.
    JobWorkflowMd,
    /// A per-job `CLAUDE.md` under `.codeless/jobs/<name>/` (only
    /// some jobs ship one; the variant is present so a future job
    /// that does can still surface patches).
    JobClaudeMd,
}

/// One patch proposal — the structured form of a single suggested
/// edit to the rulebook. Read by humans via `DOCS/SCOPE-PROPOSED.md`
/// and by automated consumers via the matching `ScopePatchProposed`
/// event.
///
/// Field invariants honoured by the *parser* (Step 5; not enforced at
/// this struct's level so a Step 4 shadow-mode emit can land with a
/// partially-populated proposal and still be observable):
///
/// - `has_predicate = true` requires `kind == Tighten` and a paired
///   predicate file in the same human commit when the patch is
///   approved.
/// - `evidence_stage_id = Some(_)` requires `kind == Loosen` and
///   names a stage whose diff exhibits the positive fixture.
/// - `kind == Tighten` ⇒ `evidence_stage_id` is `None`.
/// - `kind == Loosen` ⇒ `evidence_stage_id` is `Some(_)` and
///   `has_predicate` is `false`.
///
/// `body` carries the literal text the patch would apply (a unified
/// diff snippet, a replacement sentence, or a "delete the paragraph
/// matching ..." instruction). The Step 6 approval UX shows it
/// verbatim; the runtime does not parse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ScopePatch {
    pub id: ScopePatchId,
    /// The REVIEW stage that emitted the proposal. Pairs the proposal
    /// with the `ReviewRequested` / `ReviewApproved` envelopes the UI
    /// already renders.
    pub review_id: ReviewId,
    /// The stage that ran the REVIEW. Same value travels on the
    /// `ScopePatchProposed` event's `stage_id` field.
    pub stage_id: StageId,
    pub kind: ScopePatchKind,
    pub target: ScopePatchTarget,
    /// Repo-relative path of the file the patch proposes to edit.
    /// Step 5 cross-checks this against the mutable-set list in
    /// `rule_bearing_files`; Step 6 displays it.
    pub target_path: String,
    /// One-sentence justification the REVIEW model wrote alongside
    /// the `PASS:` sentinel. The approval UX surfaces it as the
    /// proposal's title; do not stuff prose here.
    pub rationale: String,
    /// The literal edit body (diff fragment, replacement text, or
    /// delete-this-paragraph instruction). Free-form on the wire;
    /// the human-authored approval commit interprets it.
    pub body: String,
    /// True ⇒ a predicate file landed in the same proposal. Required
    /// for `Tighten` once Step 5 enforcement is live; carried on the
    /// envelope so the approval UX can group "patch + predicate"
    /// proposals together without re-reading the body.
    pub has_predicate: bool,
    /// For `Loosen`: the stage whose diff is the positive fixture.
    /// `None` on `Tighten`; `Some` on `Loosen` once Step 5 lands.
    /// Optional in Step 4 shadow mode so partial proposals are
    /// observable on the wire and the parse-time rejection can name
    /// the missing field by Step 5.
    pub evidence_stage_id: Option<StageId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn scope_patch_id_roundtrips_through_string() {
        let id = ScopePatchId::new();
        let s = id.to_string();
        let back = ScopePatchId::from_str(&s).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn kind_serialises_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ScopePatchKind::Tighten).unwrap(),
            "\"tighten\""
        );
        assert_eq!(
            serde_json::to_string(&ScopePatchKind::Loosen).unwrap(),
            "\"loosen\""
        );
    }

    #[test]
    fn target_serialises_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ScopePatchTarget::JobScopeMd).unwrap(),
            "\"job-scope-md\""
        );
        assert_eq!(
            serde_json::to_string(&ScopePatchTarget::ClaudeMd).unwrap(),
            "\"claude-md\""
        );
    }

    #[test]
    fn full_patch_roundtrips_through_json() {
        let p = ScopePatch {
            id: ScopePatchId::new(),
            review_id: ReviewId::new(),
            stage_id: StageId::new(),
            kind: ScopePatchKind::Loosen,
            target: ScopePatchTarget::JobScopeMd,
            target_path: ".codeless/jobs/foo/SCOPE.md".into(),
            rationale: "the rule rejected a legitimate diff".into(),
            body: "delete the paragraph beginning 'Drive-by'".into(),
            has_predicate: false,
            evidence_stage_id: Some(StageId::new()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: ScopePatch = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
