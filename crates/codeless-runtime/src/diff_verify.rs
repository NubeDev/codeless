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
    // Backtick-wrapped tokens are the explicit, taught form. The
    // JOB-MODEL.md worked examples teach the agent to wrap real paths
    // in backticks; a backtick-wrapped path-shaped token is a strong
    // claim from any bullet.
    let mut rest = s;
    let mut any_backtick = false;
    while let Some(start) = rest.find('`') {
        any_backtick = true;
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let inner = after[..end].trim();
        let had_trailing_slash = inner.ends_with('/');
        if looks_path_like(inner, had_trailing_slash) {
            out.push(strip_trailing_slash(inner));
        }
        rest = &after[end + 1..];
    }

    // Bare-token extraction is the fallback for bullets that did not
    // bother with backticks. It runs ONLY when:
    //   (1) the bullet contains no backtick tokens at all (the agent
    //       did not use the taught wrapping form), AND
    //   (2) the bullet opens with a modification verb (`Wrote`,
    //       `Created`, `Edited`, …) so a verification or reading
    //       bullet does not contribute bare claims.
    // Both guards are needed: (1) prevents the "Wrote X with help
    // from Y" double-claim when X is backticked; (2) prevents a
    // "Verified X and Y" bullet from claiming either.
    if !any_backtick && bullet_claims_modification(s) {
        for raw in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')')) {
            let mut tok = raw.trim_matches(|c: char| matches!(c, '"' | '`' | '\''));
            if tok.ends_with('.') && tok[..tok.len() - 1].contains('.') {
                tok = &tok[..tok.len() - 1];
            }
            let had_trailing_slash = tok.ends_with('/');
            if looks_path_like(tok, had_trailing_slash) {
                out.push(strip_trailing_slash(tok));
            }
        }
    }
    out
}

/// First non-blank word of `s` is one of the modification verbs we
/// trust as a "this bullet claims to have changed something" signal.
/// The list is finite and explicit — extending it is a deliberate
/// choice the JOB-MODEL.md doc should also reflect.
fn bullet_claims_modification(s: &str) -> bool {
    let trimmed = s.trim_start();
    let first_word: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(
        first_word.as_str(),
        "added"
            | "wrote"
            | "created"
            | "edited"
            | "modified"
            | "touched"
            | "renamed"
            | "deleted"
            | "updated"
            | "refactored"
            | "removed"
            | "introduced"
            | "extended"
            | "ported"
            | "moved"
            | "split"
            | "merged"
            | "fixed"
            | "applied"
    )
}

fn looks_path_like(s: &str, had_trailing_slash: bool) -> bool {
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
    //
    // Branch-ref guard: a slash-bearing token with neither a dot
    // (extension) anywhere in it nor a trailing slash (directory
    // shape) is more likely a git ref (`codeless/scope-mutable-ui`,
    // `feat/foo-bar`) than a path. Real directory references either
    // include a file extension somewhere (`crates/codeless-runtime/src/x.rs`)
    // or are emitted with a trailing slash (`crates/codeless-runtime/`).
    // This rejects the branch-name case without losing real paths.
    if trimmed.contains('/') {
        if !trimmed.chars().any(|c| c.is_ascii_lowercase()) {
            return false;
        }
        if !had_trailing_slash && !trimmed.contains('.') {
            return false;
        }
        return true;
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
    if !stem.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Reject glob and brace-expansion shorthand the agent often quotes
    // from grep/find guards in WORKFLOW.md — e.g. ``.{web,desktop,
    // mobile}.tsx`` or ``*.test.tsx``. Real path tokens never contain
    // shell-glob metacharacters.
    if stem
        .chars()
        .any(|c| matches!(c, '{' | '}' | ',' | '*' | '?' | '[' | ']'))
    {
        return false;
    }
    // Reject Rust / JS / TS expression prefixes that the filename rule
    // would otherwise admit: `self.store`, `this.foo`, `Self.bar`,
    // `crate.x`, `super.y`. These are method / field accesses agents
    // routinely write in handover narration; without this guard they
    // get treated as claimed filenames named `store`, `foo`, etc.
    if matches!(stem, "self" | "this" | "Self" | "crate" | "super") {
        return false;
    }
    // Reject multi-camel-case identifiers followed by a lowercase
    // method-like extension: `MockRpcClient.emit`, `JobSpec.validate`,
    // `HttpSseClient.subscribe`. These are class.method expressions,
    // not paths. The discriminator is the count of uppercase letters
    // in the stem — single-capital stems like `Cargo.toml`,
    // `Dockerfile.linux`, `Makefile`, `README.md` are real filenames
    // and must keep passing; stems with two or more uppercase letters
    // alongside lowercase letters and no underscore / dash separator
    // are almost always PascalCase identifiers.
    let uppers = stem.chars().filter(|c| c.is_ascii_uppercase()).count();
    let has_lower = stem.chars().any(|c| c.is_ascii_lowercase());
    if uppers >= 2 && has_lower && !stem.contains('_') && !stem.contains('-') {
        return false;
    }
    true
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
    fn verification_bullets_do_not_claim_bare_paths() {
        // A bullet that opens with a reading / verification verb is
        // descriptive prose, not a claim of modification. The agent
        // mentioning the design doc it read (`Y.md`) as a bare token
        // alongside the file it wrote (`` `X.md` ``) must not surface
        // Y.md as a claimed path.
        let done = vec![
            "Wrote `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md` with resolutions for OQ#1-#6 from the workspace doc DOCS/SCOPE-MUTABLE-UI.md.".into(),
            "Verified the design doc's Dependency table is consistent with each surface's Status block.".into(),
            "Confirmed `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md` exists and reconciles the dependency table.".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec!["DOCS/SCOPE-MUTABLE-UI-DECISIONS.md".to_string()],
            "only the modification-claim bullet should contribute; \
             reading-verb bullets keep their backticked tokens but \
             must not extract bare tokens"
        );
    }

    #[test]
    fn rejects_branch_ref_shaped_tokens() {
        // A `Done` bullet that says ``committed as 79e32e9 on branch
        // `codeless/scope-mutable-ui` `` should NOT surface the branch
        // name as a claimed path. Branch refs are namespace/slug pairs
        // with no extension and no trailing slash; real directory
        // references either carry a trailing slash or include an
        // extension somewhere in the token.
        let done = vec![
            "Wrote `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md`".into(),
            "committed as 79e32e9 on branch `codeless/scope-mutable-ui`".into(),
            "remote tracking `origin/feat/foo-bar`".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec!["DOCS/SCOPE-MUTABLE-UI-DECISIONS.md".to_string()],
            "branch-ref tokens must not be extracted as claimed paths"
        );
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
    fn rejects_rust_expression_prefixes_as_paths() {
        // Realistic worker output: a Done bullet that says "drove the
        // three Todo events through `self.store.update_todo_status`".
        // The token `self.store` matches the 1-5 alpha extension rule
        // (`store` is 5 chars, all alpha) and would otherwise extract
        // as a claimed path named `store` under stem `self`. It is a
        // Rust field-access expression, not a filename.
        let done = vec![
            "added the `TodoUpdated` arm to `crates/codeless-runtime/src/stage_recorder.rs`; \
             updates flow through self.store.update_todo_status."
                .into(),
            "completed the trio via self.store and crate.bus".into(),
            "swapped Self.config for the new builder".into(),
            "this.foo and super.bar are also not paths".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec!["crates/codeless-runtime/src/stage_recorder.rs".to_string()],
            "self.X / this.X / Self.X / crate.X / super.X tokens are \
             expressions, not paths"
        );
    }

    #[test]
    fn rejects_multi_camel_case_method_expressions() {
        // The other realistic worker output: narration of a TypeScript
        // mock-client call, `MockRpcClient.emit(...)`. The 1-5 alpha
        // extension rule would otherwise admit `emit` as an extension
        // and `MockRpcClient` as the stem. Multi-camel-case stems with
        // a lowercase ext are class.method expressions.
        let done = vec![
            "wired the planner-touched envelope through `MockRpcClient.emit`".into(),
            "tested via JobSpec.validate and HttpSseClient.subscribe".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert!(
            paths.is_empty(),
            "PascalCase method-call expressions must not extract as paths, got {paths:?}"
        );
    }

    #[test]
    fn rejects_brace_expansion_and_glob_shorthand() {
        // Realistic worker output: the agent quotes a grep/find guard
        // from WORKFLOW.md verbatim in Done bullets, e.g. "verified no
        // `.{web,desktop,mobile}.tsx` shells exist". The brace-
        // expansion shorthand is not a path.
        let done = vec![
            "ran the R3 guard, no `.{web,desktop,mobile}.tsx` files present".into(),
            "checked `*.test.tsx` and `[Ff]oo.rs` patterns separately".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert!(
            paths.is_empty(),
            "shell-glob and brace-expansion tokens must not extract as paths, got {paths:?}"
        );
    }

    #[test]
    fn keeps_legitimate_single_capital_filenames() {
        // The reject-multi-camel rule must not over-fire on real
        // single-capital filenames in this repo.
        let done = vec![
            "edited `Cargo.toml`, `Dockerfile.linux`, and `README.md`".into(),
            "updated `Makefile` and a leaf `Cargo.lock`".into(),
        ];
        let paths = extract_paths_from_done(&done);
        assert_eq!(
            paths,
            vec![
                "Cargo.toml".to_string(),
                "Dockerfile.linux".to_string(),
                "README.md".to_string(),
                "Cargo.lock".to_string(),
            ],
            "single-capital stems (Cargo, Dockerfile, README) must \
             continue to extract as paths; Makefile has no extension \
             and is correctly skipped",
        );
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
