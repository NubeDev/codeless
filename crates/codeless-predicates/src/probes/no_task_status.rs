//! CLAUDE.md R2 — no task-status comments.
//!
//! The rule, in full: "Never reference stages, ticks, tickets, 'added
//! in stage 3', 'TODO from M5', 'fixed for PR #123'. The comment must
//! still make sense after the loop finishes and the branch merges."
//! Comments that pin themselves to a job's session timeline rot the
//! moment the job ends; this probe is the deterministic backstop.
//!
//! The phrase set is the four patterns called out in the rule, plus
//! the surrounding template ("fixed for ticket"). Each is matched
//! case-insensitively as a substring of a line. The probe runs against
//! source-code files only — markdown narratives about a job's stage
//! sequence are not the failure mode; comments inside code are.
//!
//! Self-skip: this file itself spells the phrases out as needles. The
//! `SELF_PREFIX` exclusion stops the probe from flagging its own
//! sources or tests.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-task-status-comments";
const SELF_PREFIX: &str = "crates/codeless-predicates/";

const CODE_EXTS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".py", ".go",
];

/// Needles are stored lowercased; the line is lowercased once per
/// iteration. "stage" + digit is the only pattern that needs a tiny
/// scanner instead of `contains`; the others are plain substrings.
const NEEDLES: &[&str] = &[
    "added in stage",
    "todo from m",
    "fixed for ticket",
    "fixed for pr #",
];

pub fn run(files: &[ChangedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in files {
        let path = norm_path(&file.path);
        if path.starts_with(SELF_PREFIX) {
            continue;
        }
        if !CODE_EXTS.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }
        for (idx, line) in file.content.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let matched =
                NEEDLES.iter().any(|n| lower.contains(n)) || contains_stage_then_digit(&lower);
            if matched {
                out.push(Violation {
                    probe: PROBE,
                    path: file.path.clone(),
                    line: Some(idx + 1),
                    message:
                        "task-status comment (stage/ticket/TODO-from reference) per CLAUDE.md R2"
                            .to_string(),
                });
            }
        }
    }
    out
}

/// True when the line contains the literal word `stage` followed by a
/// space and an ASCII digit. The rule's first example pattern is
/// "added in stage 3"; this catches the standalone form too (e.g.
/// "stage 7 cleanup"). A bare mention of the word `stage` is allowed
/// — the digit-anchor is what makes the comment a timeline pin.
fn contains_stage_then_digit(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    let needle = b"stage ";
    if bytes.len() < needle.len() + 1 {
        return false;
    }
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle && bytes[i + needle.len()].is_ascii_digit() {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(path: &str, content: &str) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            content: content.to_string(),
        }
    }

    #[test]
    fn flags_added_in_stage_phrase() {
        let files = vec![file(
            "crates/codeless-runtime/src/foo.rs",
            "// added in stage 3 — remove after launch\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn flags_bare_stage_with_digit() {
        let files = vec![file(
            "crates/codeless-runtime/src/foo.rs",
            "// stage 7 cleanup\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn flags_todo_from_milestone() {
        let files = vec![file(
            "crates/codeless-server/src/x.rs",
            "// TODO from M5: revisit this\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn flags_fixed_for_pr_reference() {
        let files = vec![file(
            "crates/codeless-server/src/x.rs",
            "// fixed for PR #123 review feedback\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn flags_fixed_for_ticket_reference() {
        let files = vec![file(
            "crates/codeless-server/src/x.rs",
            "// fixed for ticket ABC-42\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn allows_word_stage_without_digit() {
        let files = vec![file(
            "crates/codeless-runtime/src/foo.rs",
            "// runs at the staging gate before the rollout stage starts\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn ignores_markdown_files() {
        // Markdown narrative about a stage is the doc layer's job, not
        // a comment-hygiene failure.
        let files = vec![file("DOCS/notes.md", "stage 3 lands the runner\n")];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate() {
        let files = vec![file(
            "crates/codeless-predicates/src/probes/no_task_status.rs",
            "// flag added in stage N comments\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn is_case_insensitive() {
        let files = vec![file(
            "crates/codeless-runtime/src/x.rs",
            "// Added In Stage 4 — see #99\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }
}
