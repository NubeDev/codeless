//! R1 — process spawning is gated to `codeless-adapters-host`.
//!
//! The workspace `CLAUDE.md` Hard Rule R1 reads, in part: "Never import
//! `std::process` or `tokio::process` from any crate other than
//! `codeless-adapters-host`. … A grep of the source tree for
//! `process::Command` outside that crate must return zero matches."
//!
//! This probe is the enforced version of that grep. Adding it as a
//! predicate means the WORK stage cannot land a spawn helper in a
//! mobile-safe crate without the predicate runner flagging the diff.
//!
//! False-positive avoidance: this probe is itself a string-search for
//! the forbidden tokens. Scanning the predicate crate's own sources
//! would always flag the probe. Skip files under
//! `crates/codeless-predicates/` and under
//! `crates/codeless-adapters-host/`; everything else is in scope.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-process-spawn-outside-adapters-host";
const ALLOWED_PREFIX: &str = "crates/codeless-adapters-host/";
const SELF_PREFIX: &str = "crates/codeless-predicates/";

const FORBIDDEN_TOKENS: &[&str] = &[
    // Both module paths and `use` aliases for them.
    "tokio::process",
    "std::process",
];

pub fn run(files: &[ChangedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in files {
        let path = norm_path(&file.path);
        if !path.ends_with(".rs") {
            continue;
        }
        if path.starts_with(ALLOWED_PREFIX) || path.starts_with(SELF_PREFIX) {
            continue;
        }
        for (idx, line) in file.content.lines().enumerate() {
            for token in FORBIDDEN_TOKENS {
                if line.contains(token) {
                    out.push(Violation {
                        probe: PROBE,
                        path: file.path.clone(),
                        line: Some(idx + 1),
                        message: format!(
                            "{token} is gated to codeless-adapters-host per CLAUDE.md R1"
                        ),
                    });
                }
            }
        }
    }
    out
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
    fn flags_tokio_process_outside_adapters_host() {
        let files = vec![file(
            "crates/codeless-runtime/src/bad.rs",
            "use tokio::process::Command;\n",
        )];
        let violations = run(&files);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].line, Some(1));
        assert_eq!(violations[0].probe, PROBE);
    }

    #[test]
    fn flags_std_process_outside_adapters_host() {
        let files = vec![file(
            "crates/codeless-server/src/spawn.rs",
            "let _ = std::process::Command::new(\"git\");\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn allows_tokens_inside_adapters_host() {
        let files = vec![file(
            "crates/codeless-adapters-host/src/runner.rs",
            "use tokio::process::Command;\nlet c = std::process::Command::new(\"x\");\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_non_rust_files() {
        let files = vec![file(
            "docs/notes.md",
            "We sometimes invoke tokio::process in prose.\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate_to_avoid_circular_flag() {
        let files = vec![file(
            "crates/codeless-predicates/src/probes/process_spawn.rs",
            "// guard against std::process and tokio::process\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn flags_each_offending_line_separately() {
        let files = vec![file(
            "crates/codeless-runtime/src/x.rs",
            "use tokio::process::Command;\nuse std::process::exit;\n",
        )];
        let v = run(&files);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].line, Some(1));
        assert_eq!(v[1].line, Some(2));
    }
}
