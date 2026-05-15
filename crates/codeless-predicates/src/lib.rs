//! Checked-in predicate runner for stage diffs.
//!
//! The runtime invokes this crate's binary on the worktree's
//! changed-file list at the start of every REVIEW stage (Step 5 wires
//! the call). Each probe is a pure function over (path, content) pairs;
//! a violation is the predicate's deterministic answer to "does this
//! diff break a rule the rulebook already promised?".
//!
//! The crate is the *Layer-1 enforcer* in the SESSION-MUTABLE-SCOPE
//! ramp: rules that have crossed the prose-to-predicate threshold live
//! here, and a tightening `ScopePatch` lands its predicate file in the
//! same human-authored commit (per `DOCS/SESSION-MUTABLE-SCOPE-
//! DECISIONS.md` Q5). Probes therefore stay narrow on purpose — each
//! one corresponds to a single, named rule in `CLAUDE.md` or
//! `DOCS/SCOPE.md`, and the probe doc-comment cites the rule it
//! enforces.
//!
//! R1 boundary: this crate is host-only and never reaches into mobile-
//! safe crates beyond their public types. Predicates that need to run
//! `cargo` or `git` go through `codeless-adapters-host`'s exported
//! runner rather than re-implementing process spawn here (none of the
//! Step 3 seed probes need that — they scan file contents).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub mod probes;

/// A file slice handed to every probe. The runner reads the file once
/// from the worktree; probes that need to ignore the content (e.g. a
/// path-only check) can drop `content` cheaply.
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub content: String,
}

/// A single rule violation. `probe` is the probe's stable name so a
/// downstream report can group violations by rule; `line` is 1-indexed
/// when the probe can pin the offence to a line and `None` for
/// whole-file findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub probe: &'static str,
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl Violation {
    pub fn render(&self) -> String {
        match self.line {
            Some(line) => format!(
                "[{}] {}:{}: {}",
                self.probe,
                self.path.display(),
                line,
                self.message
            ),
            None => format!("[{}] {}: {}", self.probe, self.path.display(), self.message),
        }
    }
}

/// Run every seeded probe against the changed-file slice and return the
/// merged violation list. Empty list means the diff cleared every
/// predicate.
pub fn run_all(files: &[ChangedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(probes::process_spawn::run(files));
    out.extend(probes::tauri_imports::run(files));
    out.extend(probes::direct_fetch::run(files));
    out.extend(probes::no_emojis::run(files));
    out.extend(probes::no_task_status::run(files));
    out
}

/// Read a list of repo-relative paths under `worktree` into
/// [`ChangedFile`] values. Missing files (a path that the diff lists
/// because it was deleted) are skipped silently — a deleted file
/// contributes no content to scan, and the diff-verify pre-check
/// already rejected an unverified `Done` claim before this runs.
///
/// Binary-looking files (anything not valid UTF-8) are skipped with no
/// content; the probes here all reason over text, and a binary asset
/// has no meaningful "line" to flag.
pub fn read_changed(worktree: &Path, paths: &[PathBuf]) -> io::Result<Vec<ChangedFile>> {
    let mut out = Vec::with_capacity(paths.len());
    for rel in paths {
        let abs = worktree.join(rel);
        match fs::read(&abs) {
            Ok(bytes) => {
                if let Ok(content) = String::from_utf8(bytes) {
                    out.push(ChangedFile {
                        path: rel.clone(),
                        content,
                    });
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(out)
}

/// Normalise a path to forward-slash form for substring matching. The
/// probe gates ("under `crates/codeless-adapters-host/`") use string
/// prefixes; a Windows-style backslash path would silently skip the
/// gate without this normalisation.
pub(crate) fn norm_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
