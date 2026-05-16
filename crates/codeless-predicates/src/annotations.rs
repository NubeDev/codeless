//! Validator for the `enforced_by:` annotation convention.
//!
//! The convention is documented in `DOCS/SCOPE-MUTABLE-UI.md` Surface D
//! and Dependency #5: rule-bearing headings in the workspace `CLAUDE.md`
//! / `DOCS/SCOPE.md` / inner-repo `CLAUDE.md` may carry an HTML-comment
//! annotation immediately after the heading:
//!
//! ```markdown
//! ### R1 — Crate dependency direction (Rust)
//! <!-- enforced_by: crates/codeless-predicates/src/probes/process_spawn.rs -->
//! ```
//!
//! The cited path is the file that turns the prose rule into a
//! deterministic predicate. If the cited file goes missing — typically
//! because someone renamed a probe without updating the rule — the
//! annotation lies. The Surface D UI catches this as the red/warning
//! pill state; this validator catches it at CI time so the build fails
//! loudly instead of degrading silently.
//!
//! Paths are resolved against a `root` directory the caller picks. The
//! binary runs from the inner-repo root and uses inner-repo-relative
//! paths in both directions: rule files at `CLAUDE.md`, `../CLAUDE.md`,
//! `../DOCS/SCOPE.md`, with cited paths like
//! `crates/codeless-predicates/src/probes/process_spawn.rs`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PREFIX: &str = "<!-- enforced_by:";
const SUFFIX: &str = "-->";

/// One `enforced_by:` annotation found in a rule file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// Rule file the annotation appeared in (as the caller provided it).
    pub source: PathBuf,
    /// 1-indexed line number of the annotation in `source`.
    pub line: usize,
    /// Cited path, verbatim from the annotation. Resolved against
    /// `root` to check existence; not normalised otherwise.
    pub cited: PathBuf,
}

/// A broken annotation: the cited path does not resolve to a readable
/// file under `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenAnnotation {
    pub annotation: Annotation,
    pub reason: BreakReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakReason {
    /// The cited path does not exist under `root`.
    Missing,
    /// The cited path exists but is a directory.
    NotAFile,
}

impl BrokenAnnotation {
    pub fn render(&self) -> String {
        let kind = match self.reason {
            BreakReason::Missing => "missing",
            BreakReason::NotAFile => "not-a-file",
        };
        format!(
            "[enforced-by] {}:{}: cited path is {}: {}",
            self.annotation.source.display(),
            self.annotation.line,
            kind,
            self.annotation.cited.display(),
        )
    }
}

/// Extract every `<!-- enforced_by: PATH -->` annotation from a single
/// markdown source. Pure: no I/O, no normalisation beyond trimming the
/// path token.
pub fn extract(source: &Path, content: &str) -> Vec<Annotation> {
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(PREFIX) else {
            continue;
        };
        let Some(inner) = rest.strip_suffix(SUFFIX) else {
            continue;
        };
        let cited = inner.trim();
        if cited.is_empty() {
            continue;
        }
        out.push(Annotation {
            source: source.to_path_buf(),
            line: idx + 1,
            cited: PathBuf::from(cited),
        });
    }
    out
}

/// Resolve each annotation against `root` and return the subset whose
/// cited path is missing or not a regular file.
pub fn validate(annotations: &[Annotation], root: &Path) -> Vec<BrokenAnnotation> {
    let mut out = Vec::new();
    for a in annotations {
        let abs = root.join(&a.cited);
        match fs::metadata(&abs) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => out.push(BrokenAnnotation {
                annotation: a.clone(),
                reason: BreakReason::NotAFile,
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => out.push(BrokenAnnotation {
                annotation: a.clone(),
                reason: BreakReason::Missing,
            }),
            Err(err) => {
                // Treat other I/O errors (permission denied, etc.) the
                // same as Missing: the cited path is unreachable. The
                // CI signal is binary; the granularity below "broken"
                // is not load-bearing.
                let _ = err;
                out.push(BrokenAnnotation {
                    annotation: a.clone(),
                    reason: BreakReason::Missing,
                });
            }
        }
    }
    out
}

/// Read every rule file relative to `root`, extract annotations, and
/// validate them in one pass. Returns the broken-annotation list, or
/// an I/O error if a rule file is unreadable (which is itself an error
/// the caller wants to surface — a missing rule file is a bigger
/// problem than a stale annotation).
pub fn scan_rule_files(root: &Path, rule_files: &[&Path]) -> io::Result<Vec<BrokenAnnotation>> {
    let mut all = Vec::new();
    for rel in rule_files {
        let abs = root.join(rel);
        let content = fs::read_to_string(&abs)?;
        all.extend(extract(rel, &content));
    }
    Ok(validate(&all, root))
}

/// The canonical rule-file list the binary scans by default. Paths are
/// relative to the inner-repo root (where the binary runs).
pub const DEFAULT_RULE_FILES: &[&str] = &["CLAUDE.md", "../CLAUDE.md", "../DOCS/SCOPE.md"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, content: &str) {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, content).unwrap();
    }

    #[test]
    fn extract_finds_well_formed_annotation() {
        let content = "## R1 — Foo\n<!-- enforced_by: crates/x/src/probe.rs -->\nBody.\n";
        let annotations = extract(Path::new("CLAUDE.md"), content);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].line, 2);
        assert_eq!(annotations[0].cited, PathBuf::from("crates/x/src/probe.rs"));
    }

    #[test]
    fn extract_tolerates_leading_indent() {
        let content = "  <!-- enforced_by: a/b.rs -->\n";
        assert_eq!(extract(Path::new("x.md"), content).len(), 1);
    }

    #[test]
    fn extract_finds_multiple_annotations_in_one_file() {
        let content = "<!-- enforced_by: a.rs -->\n<!-- enforced_by: b.rs -->\n";
        let annotations = extract(Path::new("x.md"), content);
        assert_eq!(annotations.len(), 2);
        assert_eq!(annotations[0].line, 1);
        assert_eq!(annotations[1].line, 2);
    }

    #[test]
    fn extract_skips_html_comments_that_are_not_enforced_by() {
        let content = "<!-- TODO: tighten this -->\n<!-- enforced_by: ok.rs -->\n";
        let annotations = extract(Path::new("x.md"), content);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].cited, PathBuf::from("ok.rs"));
    }

    #[test]
    fn extract_skips_empty_path_annotations() {
        let content = "<!-- enforced_by:  -->\n";
        assert!(extract(Path::new("x.md"), content).is_empty());
    }

    #[test]
    fn validate_passes_when_cited_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "real.rs", "fn main() {}");
        let a = Annotation {
            source: PathBuf::from("CLAUDE.md"),
            line: 1,
            cited: PathBuf::from("real.rs"),
        };
        assert!(validate(&[a], dir.path()).is_empty());
    }

    #[test]
    fn validate_flags_missing_path_with_missing_reason() {
        let dir = tempfile::tempdir().unwrap();
        let a = Annotation {
            source: PathBuf::from("CLAUDE.md"),
            line: 5,
            cited: PathBuf::from("renamed_away.rs"),
        };
        let broken = validate(&[a], dir.path());
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].reason, BreakReason::Missing);
    }

    #[test]
    fn validate_flags_directory_as_not_a_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("some_dir")).unwrap();
        let a = Annotation {
            source: PathBuf::from("CLAUDE.md"),
            line: 1,
            cited: PathBuf::from("some_dir"),
        };
        let broken = validate(&[a], dir.path());
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].reason, BreakReason::NotAFile);
    }

    #[test]
    fn scan_rule_files_round_trips_extract_plus_validate() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "real.rs", "");
        write(
            dir.path(),
            "CLAUDE.md",
            "## R1\n<!-- enforced_by: real.rs -->\n## R2\n<!-- enforced_by: gone.rs -->\n",
        );
        let broken = scan_rule_files(dir.path(), &[Path::new("CLAUDE.md")]).unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].annotation.cited, PathBuf::from("gone.rs"));
        assert_eq!(broken[0].annotation.line, 4);
    }

    #[test]
    fn render_uses_rule_file_path_and_kind() {
        let broken = BrokenAnnotation {
            annotation: Annotation {
                source: PathBuf::from("CLAUDE.md"),
                line: 62,
                cited: PathBuf::from("crates/gone/foo.rs"),
            },
            reason: BreakReason::Missing,
        };
        let rendered = broken.render();
        assert!(rendered.contains("CLAUDE.md:62"));
        assert!(rendered.contains("missing"));
        assert!(rendered.contains("crates/gone/foo.rs"));
    }
}
