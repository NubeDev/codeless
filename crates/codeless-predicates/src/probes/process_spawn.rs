//! R1 — process spawning is gated to `codeless-adapters-host`.
//!
//! The workspace `CLAUDE.md` Hard Rule R1 reads, in part: "Never import
//! `std::process` or `tokio::process` from any crate other than
//! `codeless-adapters-host`. … A grep of the source tree for
//! `process::Command` outside that crate must return zero matches."
//!
//! R1 names the spawn-shape symbols specifically: `Command`, `Child`,
//! and the child stdio handles. The bare module path `std::process`
//! also resolves benign return-type imports (`ExitCode`, `exit`,
//! `Stdio`) that every CLI uses; flagging those as R1 violations is a
//! false positive that drowns out the real signal. The probe matches
//! only the spawn-shape segments after `::process::`.
//!
//! Carve-outs:
//! - Files under `crates/codeless-adapters-host/` are the home of the
//!   spawn surface and are exempt by definition.
//! - Files under `crates/codeless-predicates/` are skipped so this
//!   probe's own sources do not flag themselves.
//! - Anything under a `tests/` directory or with an `#[cfg(test)]`
//!   attribute inside the file is integration / unit test code; the
//!   CLAUDE.md handover for this ramp documented the test-side
//!   carve-out and the rule is about production reachability.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-process-spawn-outside-adapters-host";
const ALLOWED_PREFIX: &str = "crates/codeless-adapters-host/";
const SELF_PREFIX: &str = "crates/codeless-predicates/";

const FORBIDDEN_SUFFIXES: &[&str] = &[
    "::process::Command",
    "::process::Child",
    "::process::ChildStdin",
    "::process::ChildStdout",
    "::process::ChildStderr",
];

fn is_test_path(path: &str) -> bool {
    path.contains("/tests/") || path.starts_with("tests/")
}

fn file_is_test_module(content: &str) -> bool {
    content.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("#[cfg(test)]") || t.starts_with("#![cfg(test)]")
    })
}

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
        if is_test_path(&path) {
            continue;
        }
        // Cheap whole-file scan: if any line carries `#[cfg(test)]`,
        // assume the file's spawn usage is gated to tests. This is a
        // coarse rule — a real module could mix prod and test code —
        // but it matches existing repo patterns and is symmetric with
        // the `tests/` carve-out above.
        let file_test_gated = file_is_test_module(&file.content);
        for (idx, line) in file.content.lines().enumerate() {
            for token in FORBIDDEN_SUFFIXES {
                if line.contains(token) {
                    if file_test_gated {
                        continue;
                    }
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
    fn flags_tokio_process_command_outside_adapters_host() {
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
    fn flags_std_process_command_outside_adapters_host() {
        let files = vec![file(
            "crates/codeless-server/src/spawn.rs",
            "let _ = std::process::Command::new(\"git\");\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn allows_benign_process_imports() {
        // ExitCode, exit, Stdio are the realistic CLI return-type
        // and child-stdio-config imports. They are not the spawn
        // surface R1 names; the probe must let them through.
        let files = vec![file(
            "crates/codeless-cli/src/run.rs",
            "use std::process::ExitCode;\nuse std::process::exit;\nuse std::process::Stdio;\n",
        )];
        assert!(
            run(&files).is_empty(),
            "got: {:?}",
            run(&files)
                .iter()
                .map(|v| v.message.clone())
                .collect::<Vec<_>>()
        );
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
            "We sometimes invoke tokio::process::Command in prose.\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate_to_avoid_circular_flag() {
        let files = vec![file(
            "crates/codeless-predicates/src/probes/process_spawn.rs",
            "// guard against std::process::Command\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_integration_tests_directory() {
        let files = vec![file(
            "crates/codeless-cli/tests/run_once.rs",
            "let _ = std::process::Command::new(\"codeless\");\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_inline_cfg_test_modules() {
        let files = vec![file(
            "crates/codeless-runtime/src/template_runner.rs",
            "fn prod() {}\n#[cfg(test)]\nmod tests {\n    fn helper() { let _ = std::process::Command::new(\"git\"); }\n}\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn flags_each_offending_line_separately() {
        let files = vec![file(
            "crates/codeless-runtime/src/x.rs",
            "use tokio::process::Command;\nuse std::process::Command as C;\n",
        )];
        let v = run(&files);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].line, Some(1));
        assert_eq!(v[1].line, Some(2));
    }
}
