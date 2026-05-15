//! Layer-1 diff-verify pre-check for REVIEW stages
//! (SESSION-MUTABLE-SCOPE Step 2).
//!
//! Before a REVIEW stage's model prompt runs, the runtime walks every
//! path mentioned in the prior WORK stage's handover `Done` section
//! and confirms that path appears in the worktree's git diff. A path
//! claimed in `Done` that no commit touched is the canonical
//! "session hallucinated its own output" failure mode; catching it
//! deterministically — no tokens, no model — is the highest-EV check
//! in the entire ramp.
//!
//! This module is the pure logic:
//!
//! - [`extract_paths_from_done`] turns a list of `Done` bullets into
//!   the set of plausible file paths the bullets claim were touched.
//! - [`verify_paths_in_diff`] confirms every claimed path matches an
//!   entry in the worktree's changed-files list (exact match, or path
//!   suffix — a bullet that names `template_runner.rs` matches a diff
//!   entry of `crates/codeless-runtime/src/template_runner.rs`).
//!
//! Spawning `git` to populate the diff list lives in
//! `codeless-adapters-host::git_changed` per the workspace's R1
//! (process spawning gated to the adapters crate); `template_runner`
//! is the only caller and it stitches the two together.
//!
//! Auto-FAIL semantics: any verification miss is the REVIEW stage's
//! verdict before the model is invoked. The handover author's claims
//! are the contract; the runtime enforces the contract; a model that
//! never sees the failed pre-check cannot be asked to wave it through.

use std::collections::BTreeSet;

use codeless_types::Handover;

/// A path the handover claimed was touched but which the diff does not
/// include. The `claimed` form is exactly what the bullet wrote (so the
/// failure message points the model at its own text); `candidates` are
/// the closest diff entries by suffix, surfaced to help the human
/// reviewer triage a near-miss without re-running git locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPath {
    pub claimed: String,
    pub candidates: Vec<String>,
}

/// Verdict of a diff-verify pre-check. Variants are explicit so the
/// caller can log the success path (the set of paths it confirmed) and
/// the failure path (the misses) without re-deriving them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffVerifyOutcome {
    /// Every claimed path resolved to a diff entry. `verified` is the
    /// set of unique paths that were checked.
    Pass { verified: Vec<String> },
    /// At least one claimed path is absent from the diff. `missing`
    /// lists every miss so the message can name them all rather than
    /// fail-fast on the first one.
    Fail { missing: Vec<MissingPath> },
    /// The handover's `Done` named no path-shaped tokens. The runtime
    /// treats this as a PASS (there is nothing to verify) but keeps
    /// the variant distinct so the caller can decide whether to log a
    /// warning — `Done` bullets that name nothing concrete are the
    /// anti-example the JOB-MODEL.md `Done` section explicitly calls
    /// out.
    NothingToVerify,
}

/// Extract path-shaped tokens from a handover's `Done` bullets.
///
/// Two layered heuristics:
///
/// 1. Anything wrapped in backticks is a candidate. The JOB-MODEL.md
///    worked examples uniformly wrap paths in backticks; the model
///    that follows the example will land in this branch.
/// 2. Failing backticks, an unquoted whitespace-delimited token is a
///    candidate when it either contains a forward slash (suggesting a
///    directory path) or matches `filename.ext` shape with a small
///    extension (so `template_runner.rs` and `template.yaml` are
///    extracted, but `1.5` or `R1.` are not).
///
/// The output is deduped and order-preserving across bullets so the
/// failure message can point at the first miss in the first bullet
/// that introduced it.
pub fn extract_paths_from_done(done: &[String]) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for bullet in done {
        for tok in path_candidates_in(bullet) {
            if seen.insert(tok.clone()) {
                out.push(tok);
            }
        }
    }
    out
}

fn path_candidates_in(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Backtick-wrapped tokens first; bullets that mix backticked and
    // bare tokens (uncommon but legal) still see both branches.
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let inner = after[..end].trim();
        if looks_path_like(inner) {
            out.push(strip_trailing_slash(inner));
        }
        rest = &after[end + 1..];
    }

    // Bare tokens: split on whitespace and a handful of trailing-
    // punctuation characters (`,`, `;`, `.` only when not part of an
    // extension — handled by the looks_path_like filter).
    for raw in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')')) {
        // Strip a trailing sentence period only if the rest still has
        // an internal dot (so `a.txt.` becomes `a.txt`, but `1.` is
        // discarded by `looks_path_like`).
        let mut tok = raw.trim_matches(|c: char| matches!(c, '"' | '`' | '\''));
        if tok.ends_with('.') && tok[..tok.len() - 1].contains('.') {
            tok = &tok[..tok.len() - 1];
        }
        if looks_path_like(tok) {
            out.push(strip_trailing_slash(tok));
        }
    }
    out
}

fn looks_path_like(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Whitespace inside the token means we are looking at quoted prose,
    // not a path. Commit subjects with a trailing `.py` / `.rs` /
    // `.md` are the realistic false positive: a `Done` bullet that
    // reads ``committed as `WORK fix bar.rs` `` should not extract
    // the whole quoted phrase as a claimed path.
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    // Reject anything that starts with punctuation; real paths start
    // with an ASCII letter, digit, dot (`./foo`) or slash. The model
    // never produces a leading slash in our examples but we accept it
    // defensively because git's own diff output may.
    let first = s.chars().next().unwrap();
    if !(first.is_ascii_alphanumeric() || first == '.' || first == '/' || first == '_') {
        return false;
    }
    // Strip any trailing slash before evaluating the extension rule.
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    // Path-segment rule: containing a slash counts when the token
    // also contains at least one lowercase ASCII letter. The
    // lowercase requirement rejects tokens like `PASS/FAIL` (which
    // appear inside REVIEW prompts and bullet prose) without
    // excluding real codebase paths — every directory in this repo
    // contains lowercase letters.
    if trimmed.contains('/') {
        return trimmed.chars().any(|c| c.is_ascii_lowercase());
    }
    // Filename rule: `name.ext` where `ext` is 1-5 ASCII alpha chars.
    // Rejects `1.5`, `R4.`, `etc.`, version-y strings, and bare
    // identifiers (`SCOPE` without an extension is not a path).
    let (stem, ext) = match trimmed.rsplit_once('.') {
        Some(parts) => parts,
        None => return false,
    };
    if stem.is_empty() {
        return false;
    }
    if !(1..=5).contains(&ext.len()) || !ext.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Stem must contain at least one alpha char so `123.txt` doesn't
    // sneak through but `a1.txt` does.
    stem.chars().any(|c| c.is_ascii_alphabetic())
}

fn strip_trailing_slash(s: &str) -> String {
    s.trim_end_matches('/').to_owned()
}

/// Verify that every `claimed` path is matched by an entry in
/// `diff_paths`. A diff entry matches when it is exactly equal or
/// when the diff entry's path ends with `/<claimed>` (so the model
/// naming a leaf — `template_runner.rs` — succeeds when the diff
/// reports the fully-qualified path).
pub fn verify_paths_in_diff(claimed: &[String], diff_paths: &[String]) -> DiffVerifyOutcome {
    if claimed.is_empty() {
        return DiffVerifyOutcome::NothingToVerify;
    }
    let mut missing: Vec<MissingPath> = Vec::new();
    let mut verified: Vec<String> = Vec::new();
    for c in claimed {
        if diff_paths.iter().any(|d| paths_match(c, d)) {
            verified.push(c.clone());
            continue;
        }
        // Build a small candidates list for the failure message — diff
        // entries that share the basename are the most likely intended
        // target. Limit to three to keep the message readable.
        let basename = leaf_of(c);
        let mut candidates: Vec<String> = diff_paths
            .iter()
            .filter(|d| !basename.is_empty() && leaf_of(d) == basename)
            .cloned()
            .collect();
        candidates.truncate(3);
        missing.push(MissingPath {
            claimed: c.clone(),
            candidates,
        });
    }
    if missing.is_empty() {
        DiffVerifyOutcome::Pass { verified }
    } else {
        DiffVerifyOutcome::Fail { missing }
    }
}

fn paths_match(claimed: &str, diff_entry: &str) -> bool {
    if claimed == diff_entry {
        return true;
    }
    // Suffix match: a bullet that names a leaf must match a diff
    // entry whose path ends with `/<claimed>`. We do not allow the
    // reverse (claimed-ends-with-diff) because that loosens to false
    // matches — a claim of `runtime/src/template_runner.rs` should
    // not be satisfied by a diff entry of `template_runner.rs` in
    // some unrelated crate.
    diff_entry.ends_with(&format!("/{claimed}"))
}

fn leaf_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Convenience entry point: take a handover and the diff path list,
/// run extraction + verification, return the outcome.
pub fn verify_handover(handover: &Handover, diff_paths: &[String]) -> DiffVerifyOutcome {
    let claimed = extract_paths_from_done(&handover.done);
    verify_paths_in_diff(&claimed, diff_paths)
}

/// Render a one-line `FAIL:` reason for the `Fail` outcome, suitable
/// for the runtime's `RunnerOutcome::Failed` reason and the structured
/// log. Kept here so the caller doesn't reimplement the format and
/// drift on punctuation.
pub fn fail_reason(missing: &[MissingPath]) -> String {
    let mut s = String::from("handover `Done` claims paths not in the diff: ");
    let names: Vec<String> = missing.iter().map(|m| m.claimed.clone()).collect();
    s.push_str(&names.join(", "));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_backticked_paths() {
        let done = vec![
            "updated `crates/codeless-runtime/src/template_runner.rs` to parse PASS/FAIL".into(),
            "added `crates/codeless-runtime/src/diff_verify.rs`".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec![
                "crates/codeless-runtime/src/template_runner.rs".to_string(),
                "crates/codeless-runtime/src/diff_verify.rs".to_string(),
            ]
        );
    }

    #[test]
    fn extracts_bare_paths_with_slashes_and_extensions() {
        // JOB-MODEL.md's anti-example (the validator-rejection worked
        // example) writes the path bare. The extractor must catch it
        // so the rejection message is accurate.
        let done = vec![
            "updated crates/codeless-runtime/src/template_runner.rs to parse the sentinel".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec!["crates/codeless-runtime/src/template_runner.rs".to_string()]
        );
    }

    #[test]
    fn ignores_numeric_dotted_tokens() {
        // Sentence-final periods, version numbers, and abbreviations
        // are not paths. The R-numbers in the prose (R1, R4) are not
        // paths either.
        let done = vec![
            "addressed R1 and R4; bumped MSRV to 1.78 in workspace TOML.".into(),
            "test count is now 1.5x prior; investigate later.".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert!(paths.is_empty(), "expected no paths, got {paths:?}");
    }

    #[test]
    fn extracts_directory_reference() {
        let done = vec!["seeded `crates/codeless-predicates/` as a new member".into()];
        let paths = extract_paths_from_done(&done);
        assert_eq!(paths, vec!["crates/codeless-predicates".to_string()]);
    }

    #[test]
    fn rejects_backticked_commit_subjects_that_end_in_a_filename() {
        // Realistic worker output: a bullet that says
        // ``committed as `WORK add foo: add bar.py` `` should NOT
        // surface `WORK add foo: add bar.py` as a claimed path.
        // The whitespace inside the backtick disqualifies the token.
        let done = vec![
            "created `bar.py` exporting `bar(name)`".into(),
            "committed as `WORK add greet: add bar.py`".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec!["bar.py".to_string()],
            "quoted commit subject must not be extracted as a path"
        );
    }

    #[test]
    fn deduplicates_repeated_paths_across_bullets() {
        let done = vec![
            "edited `a/b.rs`".into(),
            "and then edited a/b.rs again to fix the test".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(paths, vec!["a/b.rs".to_string()]);
    }

    #[test]
    fn verify_passes_when_every_claim_matches_exactly() {
        let claimed = vec!["a/b.rs".to_string(), "c/d.rs".to_string()];
        let diff = vec!["a/b.rs".to_string(), "c/d.rs".to_string(), "e/f.rs".into()];
        match verify_paths_in_diff(&claimed, &diff) {
            DiffVerifyOutcome::Pass { verified } => assert_eq!(verified.len(), 2),
            other => panic!("expected Pass, got {other:?}"),
        }
    }

    #[test]
    fn verify_passes_when_claim_is_a_suffix_of_a_diff_entry() {
        // The model wrote the leaf only; the diff knows the full path.
        let claimed = vec!["template_runner.rs".to_string()];
        let diff = vec!["crates/codeless-runtime/src/template_runner.rs".to_string()];
        match verify_paths_in_diff(&claimed, &diff) {
            DiffVerifyOutcome::Pass { .. } => {}
            other => panic!("expected Pass, got {other:?}"),
        }
    }

    #[test]
    fn verify_does_not_match_in_reverse() {
        // A fully-qualified claim must match the fully-qualified diff
        // entry. A diff entry of a bare leaf in some other crate must
        // not satisfy a deep claim.
        let claimed = vec!["runtime/src/template_runner.rs".to_string()];
        let diff = vec!["template_runner.rs".to_string()];
        match verify_paths_in_diff(&claimed, &diff) {
            DiffVerifyOutcome::Fail { missing } => {
                assert_eq!(missing.len(), 1);
                assert_eq!(missing[0].claimed, "runtime/src/template_runner.rs");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verify_reports_missing_with_basename_candidates() {
        let claimed = vec!["wrong_dir/template_runner.rs".to_string()];
        let diff = vec![
            "crates/codeless-runtime/src/template_runner.rs".to_string(),
            "unrelated/file.rs".to_string(),
        ];
        match verify_paths_in_diff(&claimed, &diff) {
            DiffVerifyOutcome::Fail { missing } => {
                assert_eq!(missing.len(), 1);
                assert_eq!(
                    missing[0].candidates,
                    vec!["crates/codeless-runtime/src/template_runner.rs".to_string()]
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verify_empty_claim_set_is_nothing_to_verify() {
        let outcome = verify_paths_in_diff(&[], &["a/b.rs".to_string()]);
        assert_eq!(outcome, DiffVerifyOutcome::NothingToVerify);
    }

    #[test]
    fn verify_handover_threads_end_to_end_through_extraction() {
        let h = Handover {
            done: vec![
                "updated `a/b.rs` and added `c/d.rs`".into(),
                "noted in `unrelated/notes.md`".into(),
            ],
            next: vec!["go".into()],
            ..Default::default()
        };
        let diff = vec![
            "a/b.rs".into(),
            "c/d.rs".into(),
            // notes.md missing — verifier must report it.
        ];
        match verify_handover(&h, &diff) {
            DiffVerifyOutcome::Fail { missing } => {
                let claimed: Vec<&str> = missing.iter().map(|m| m.claimed.as_str()).collect();
                assert_eq!(claimed, vec!["unrelated/notes.md"]);
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn fail_reason_lists_every_missing_path() {
        let missing = vec![
            MissingPath {
                claimed: "a/b.rs".into(),
                candidates: vec![],
            },
            MissingPath {
                claimed: "c/d.rs".into(),
                candidates: vec![],
            },
        ];
        let r = fail_reason(&missing);
        assert!(r.contains("a/b.rs"));
        assert!(r.contains("c/d.rs"));
        assert!(r.contains("not in the diff"));
    }
}
