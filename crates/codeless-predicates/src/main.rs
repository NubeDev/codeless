//! `codeless-predicates` binary — xtask-shaped entry point. Reads the
//! worktree root and a newline-delimited list of changed paths, runs
//! every seeded probe, prints violations one per line, exits non-zero
//! if any probe flagged the diff.
//!
//! Invocation shape (Step 5 wires this into the runtime; the CLI
//! contract is fixed now so the integration is a one-line shell
//! pipeline):
//!
//! ```text
//! git diff --name-only base...HEAD | codeless-predicates --worktree /path
//! ```
//!
//! Exit codes: 0 = clean, 1 = at least one violation, 2 = I/O or usage
//! error. The runtime distinguishes "rule broken" from "tooling broken"
//! by the code; a 2 must not be reported as a FAIL verdict.

use std::env;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use codeless_predicates::annotations::{scan_rule_files, DEFAULT_RULE_FILES};
use codeless_predicates::{read_changed, run_all};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut worktree: Option<PathBuf> = None;
    let mut validate_annotations = false;
    let mut root: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--worktree" => {
                worktree = args.next().map(PathBuf::from);
            }
            "--validate-annotations" => {
                validate_annotations = true;
            }
            "--root" => {
                root = args.next().map(PathBuf::from);
            }
            "-h" | "--help" => {
                print_help();
                return ExitCode::from(0);
            }
            other => {
                eprintln!("codeless-predicates: unknown argument: {other}");
                print_help();
                return ExitCode::from(2);
            }
        }
    }

    if validate_annotations {
        return run_validate_annotations(root.as_deref());
    }

    let Some(worktree) = worktree else {
        eprintln!("codeless-predicates: --worktree <path> is required");
        return ExitCode::from(2);
    };

    let paths = match read_paths_stdin() {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("codeless-predicates: failed to read paths from stdin: {err}");
            return ExitCode::from(2);
        }
    };

    let files = match read_changed(&worktree, &paths) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("codeless-predicates: failed to read changed files: {err}");
            return ExitCode::from(2);
        }
    };

    let violations = run_all(&files);
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for v in &violations {
        let _ = writeln!(out, "{}", v.render());
    }
    if violations.is_empty() {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    }
}

fn read_paths_stdin() -> io::Result<Vec<PathBuf>> {
    let stdin = io::stdin();
    let mut out = Vec::new();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(PathBuf::from(trimmed));
    }
    Ok(out)
}

fn run_validate_annotations(root: Option<&Path>) -> ExitCode {
    let root = match root {
        Some(p) => p.to_path_buf(),
        None => match env::current_dir() {
            Ok(p) => p,
            Err(err) => {
                eprintln!("codeless-predicates: failed to read current dir: {err}");
                return ExitCode::from(2);
            }
        },
    };
    let files: Vec<&Path> = DEFAULT_RULE_FILES.iter().map(Path::new).collect();
    let broken = match scan_rule_files(&root, &files) {
        Ok(b) => b,
        Err(err) => {
            eprintln!(
                "codeless-predicates: failed to scan rule files under {}: {err}",
                root.display()
            );
            return ExitCode::from(2);
        }
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for b in &broken {
        let _ = writeln!(out, "{}", b.render());
    }
    if broken.is_empty() {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    }
}

fn print_help() {
    eprintln!(
        "codeless-predicates --worktree <PATH>\n\
         codeless-predicates --validate-annotations [--root <PATH>]\n\
         \n\
         Default form: reads newline-delimited changed paths from stdin\n\
         (relative to <PATH>) and runs every checked-in predicate against\n\
         the current file contents.\n\
         \n\
         --validate-annotations: scans the workspace rule files for\n\
         `<!-- enforced_by: PATH -->` comments and verifies each cited\n\
         path resolves to a file under --root (defaulting to the current\n\
         directory). Catches a predicate being renamed without the rule\n\
         being updated.\n\
         \n\
         Exit codes:\n\
           0  no violations / no broken annotations\n\
           1  at least one rule was violated / annotation is broken\n\
           2  usage / I/O error"
    );
}
