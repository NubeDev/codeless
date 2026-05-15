//! Layer-1 file-set classifier — the canonical list of "rule-bearing"
//! files that a WORK stage's diff is not allowed to touch.
//!
//! Why a separate module: the rulebook (SCOPE.md, CLAUDE.md, the
//! per-job WORKFLOW/SCOPE under `.codeless/jobs/<name>/`, the
//! checked-in predicate sources) is the system's contract with itself.
//! WORK editing the rulebook is exactly the failure mode this whole
//! ramp exists to prevent — and wire-format files are a stricter
//! superset, since those change via `schema_version` bumps and
//! migrations rather than REVIEW patches.
//!
//! Scope: this module is **data + a classifier**. Step 2 (diff-verify)
//! wires it into the per-stage pre-check. Step 5 splits the rulebook
//! set into the "mutable via REVIEW" sub-set vs the "schema-bump
//! only" sub-set. Today the classifier returns the broad bucket; the
//! sub-sets refine `FileClass::Rulebook` without changing the
//! WORK-cannot-touch guarantee.

use std::path::{Component, Path};

/// Bucket for a file path relative to the repo root. Order in
/// `classify` is strict-first-wins: a path that matches multiple
/// buckets (e.g. `crates/codeless-types/src/handover.rs` is both a
/// source file and a wire-format file) is reported as the most
/// constrained class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    /// Wire-format files. Changing one is a schema break; even a
    /// REVIEW patch is not allowed to touch these — the migration
    /// path is a `schema_version` bump in `codeless-types` plus a
    /// human-authored migration. WORK touching one is auto-FAIL.
    WireFormat,
    /// The patch-proposal queue. REVIEW stages may append; the
    /// approval CLI may remove an approved entry; WORK touching it
    /// is auto-FAIL (per `SESSION-MUTABLE-SCOPE-DECISIONS.md` Q1).
    /// Listed here for completeness — the file itself lands with
    /// Step 4 of the ramp.
    ReviewQueue,
    /// The rulebook itself: `SCOPE.md`, `CLAUDE.md`, per-job
    /// `WORKFLOW.md`, and the checked-in predicate sources. WORK
    /// touching any of these is auto-FAIL; REVIEW touches them only
    /// indirectly by proposing a `ScopePatch`, never by editing the
    /// file in place (Step 4+).
    Rulebook,
    /// Everything else. WORK writes freely; REVIEW writes freely
    /// (subject to the per-job WORKFLOW.md guidance, not enforced
    /// here).
    Open,
}

/// Wire-format file paths — exact match against the path relative to
/// the repo root. Kept as an explicit list rather than a glob so a
/// renamed file shows up as a code change requiring a deliberate
/// update to this list; a glob would silently drift.
const WIRE_FORMAT_FILES: &[&str] = &[
    "DOCS/JOB-MODEL.md",
    "DOCS/JOB-LOOP.md",
    "crates/codeless-types/src/handover.rs",
];

/// The review queue file. Step 4 of the ramp lands the file itself;
/// the classifier knows the path now so Step 2's diff-verify can
/// already cite the right bucket if the file appears.
const REVIEW_QUEUE_FILES: &[&str] = &["DOCS/SCOPE-PROPOSED.md"];

/// Rulebook file names that match by **basename**, regardless of
/// directory. Captures the workspace-level `CLAUDE.md` and the inner
/// `codeless/CLAUDE.md`, as well as every `SCOPE.md` / `WORKFLOW.md`
/// under `.codeless/jobs/<name>/`. Listed by basename rather than by
/// full path because a new job's `.codeless/jobs/<name>/SCOPE.md` is
/// rule-bearing the moment it lands — encoding the per-job names
/// here would force this module to be edited every time a job
/// scaffolds.
const RULEBOOK_BASENAMES: &[&str] = &["SCOPE.md", "CLAUDE.md", "WORKFLOW.md", "CODELESS.md"];

/// Rulebook directory prefixes — any file under one of these paths
/// (relative to repo root) is treated as rule-bearing. The predicate
/// crate's sources are code, but per `SESSION-MUTABLE-SCOPE-DECISIONS.md`
/// Q5 they change only through a human-authored commit alongside an
/// approved loosening patch; WORK editing them as a side effect of an
/// unrelated stage is exactly the failure mode the file-set rule
/// prevents.
const RULEBOOK_DIR_PREFIXES: &[&str] = &["crates/codeless-predicates/src"];

/// Rulebook exact paths — files that are rule-bearing by location
/// rather than by basename. The decisions doc itself is rule-bearing
/// because future stages cite it: a WORK stage silently editing a Q
/// answer would change the contract every later stage was authored
/// against.
const RULEBOOK_EXACT_FILES: &[&str] = &[
    "DOCS/SESSION-MUTABLE-SCOPE.md",
    "DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md",
];

/// Classify a repo-relative path into one of the four buckets.
///
/// The input is normalised: backslashes are converted to forward
/// slashes, a leading `./` is stripped, and `.` / `..` components are
/// left alone (a path containing `..` is suspicious in its own right
/// and the caller — diff-verify — should reject it independently;
/// this classifier does not silently resolve up-traversal).
pub fn classify(path: &Path) -> FileClass {
    let normalised = normalise(path);

    if WIRE_FORMAT_FILES.iter().any(|p| *p == normalised) {
        return FileClass::WireFormat;
    }
    if REVIEW_QUEUE_FILES.iter().any(|p| *p == normalised) {
        return FileClass::ReviewQueue;
    }
    if RULEBOOK_EXACT_FILES.iter().any(|p| *p == normalised) {
        return FileClass::Rulebook;
    }
    if RULEBOOK_DIR_PREFIXES
        .iter()
        .any(|prefix| normalised == *prefix || normalised.starts_with(&format!("{prefix}/")))
    {
        return FileClass::Rulebook;
    }
    let basename = Path::new(&normalised)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if RULEBOOK_BASENAMES.contains(&basename) {
        return FileClass::Rulebook;
    }
    FileClass::Open
}

/// Convenience: `true` when a WORK stage is permitted to modify the
/// file. Step 2 (diff-verify) compares this against the stage's
/// changed-file list; any `false` aborts the stage.
pub fn work_may_touch(path: &Path) -> bool {
    matches!(classify(path), FileClass::Open)
}

fn normalise(path: &Path) -> String {
    let mut out = String::new();
    for (i, comp) in path.components().enumerate() {
        let part = match comp {
            Component::Normal(s) => s.to_string_lossy().to_string(),
            Component::CurDir => continue,
            Component::ParentDir => "..".to_string(),
            Component::RootDir => continue,
            Component::Prefix(p) => p.as_os_str().to_string_lossy().to_string(),
        };
        if i > 0 && !out.is_empty() {
            out.push('/');
        }
        out.push_str(&part);
    }
    out.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cls(s: &str) -> FileClass {
        classify(&PathBuf::from(s))
    }

    #[test]
    fn wire_format_paths_are_wire_format() {
        assert_eq!(cls("DOCS/JOB-MODEL.md"), FileClass::WireFormat);
        assert_eq!(cls("DOCS/JOB-LOOP.md"), FileClass::WireFormat);
        assert_eq!(
            cls("crates/codeless-types/src/handover.rs"),
            FileClass::WireFormat
        );
    }

    #[test]
    fn review_queue_path_is_review_queue() {
        assert_eq!(cls("DOCS/SCOPE-PROPOSED.md"), FileClass::ReviewQueue);
    }

    #[test]
    fn rulebook_basenames_match_anywhere() {
        // Repo-root rulebook files.
        assert_eq!(cls("CLAUDE.md"), FileClass::Rulebook);
        assert_eq!(cls("CODELESS.md"), FileClass::Rulebook);
        // Per-job rulebook files.
        assert_eq!(
            cls(".codeless/jobs/session-mutable-scope/SCOPE.md"),
            FileClass::Rulebook
        );
        assert_eq!(
            cls(".codeless/jobs/session-mutable-scope/WORKFLOW.md"),
            FileClass::Rulebook
        );
    }

    #[test]
    fn predicate_crate_sources_are_rulebook() {
        assert_eq!(
            cls("crates/codeless-predicates/src/lib.rs"),
            FileClass::Rulebook
        );
        assert_eq!(
            cls("crates/codeless-predicates/src/probes/clippy.rs"),
            FileClass::Rulebook
        );
    }

    #[test]
    fn decisions_doc_is_rulebook() {
        assert_eq!(
            cls("DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md"),
            FileClass::Rulebook
        );
        assert_eq!(cls("DOCS/SESSION-MUTABLE-SCOPE.md"), FileClass::Rulebook);
    }

    #[test]
    fn open_paths_classify_as_open() {
        assert_eq!(
            cls("crates/codeless-runtime/src/template_runner.rs"),
            FileClass::Open
        );
        assert_eq!(cls("README.md"), FileClass::Open);
        assert_eq!(cls("ui/codeless-ui/src/main.tsx"), FileClass::Open);
    }

    #[test]
    fn wire_format_outranks_rulebook_for_handover_rs() {
        // `handover.rs` is both a source file and a wire-format file;
        // wire-format is the more constrained bucket so the classifier
        // must report it.
        assert_eq!(
            cls("crates/codeless-types/src/handover.rs"),
            FileClass::WireFormat
        );
    }

    #[test]
    fn work_may_touch_open_files_only() {
        assert!(work_may_touch(&PathBuf::from("README.md")));
        assert!(!work_may_touch(&PathBuf::from("CLAUDE.md")));
        assert!(!work_may_touch(&PathBuf::from("DOCS/JOB-MODEL.md")));
        assert!(!work_may_touch(&PathBuf::from("DOCS/SCOPE-PROPOSED.md")));
        assert!(!work_may_touch(&PathBuf::from(
            "crates/codeless-predicates/src/foo.rs"
        )));
    }

    #[test]
    fn leading_dot_slash_is_stripped() {
        assert_eq!(cls("./CLAUDE.md"), FileClass::Rulebook);
        assert_eq!(cls("./DOCS/JOB-MODEL.md"), FileClass::WireFormat);
    }

    #[test]
    fn similar_basenames_do_not_false_match() {
        // `SCOPE-PROPOSED.md` is a different file; classification falls
        // through to ReviewQueue (already covered) but a near-miss like
        // `SCOPE-NOTES.md` must remain Open so unrelated docs aren't
        // accidentally locked.
        assert_eq!(cls("DOCS/SCOPE-NOTES.md"), FileClass::Open);
        // `CLAUDE-old.md` is not the rulebook.
        assert_eq!(cls("docs/CLAUDE-old.md"), FileClass::Open);
    }
}
