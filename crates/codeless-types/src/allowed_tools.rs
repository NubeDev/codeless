//! Allowed-tools pattern syntax — the one matching rule a thread's
//! capability list speaks.
//!
//! The substrate doc (`DOCS/PLUGIN-SUBSTRATE.md` item 3) pins this rule:
//! every entry in a persona's `allowed_tools` list is either a literal
//! tool id (`fs.read`) or a dotted-prefix glob ending in `.*`
//! (`estimate.*`). No shell globbing, no regex, no `*` anywhere but the
//! tail. A tool id matches an entry iff (a) it equals the literal, or
//! (b) the entry ends in `.*` and the tool id starts with the entry's
//! prefix plus a dot.
//!
//! Centralised here, in `codeless-types`, because three callers consume
//! it from different layers of the dep graph: the plugin manifest
//! reader (item 6, lives in `codeless-tools`) validates patterns at
//! load time, the runtime's chat path (PS3, this stage) validates and
//! matches when it derives a thread's effective tool set, and the
//! mobile-safe `codeless-client` will eventually surface the same list
//! to UI badges. Putting the rule in `codeless-types` keeps every
//! caller in lockstep — there is no second implementation that can
//! drift from the spec.
//!
//! The module is pure: no I/O, no `tokio`, no `serde` derives on the
//! function surface. Patterns are just `&str` in / `bool` out so the
//! call sites stay free of allocation in the hot path.

use std::fmt;

/// Reasons a pattern fails the substrate-doc syntax. The display impl
/// is the message surfaced by the manifest reader when it rejects a
/// `plugin.toml` at load time, and by the runtime's persona-validation
/// path when a malformed pattern lands on a thread row. Kept as a
/// concrete enum (no `String` payload) so call sites can match for
/// telemetry without parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowedToolPatternError {
    /// The pattern was the empty string.
    Empty,
    /// The pattern contained a `*` somewhere other than the trailing
    /// `.*` (e.g. `est*mate`, `*.read`, `estimate.*.bom`). Only the
    /// dotted-prefix suffix form is legal.
    StarOutsideTail,
    /// The pattern contained a regex / shell-glob metacharacter the
    /// rule excludes outright (`?`, `[`, `]`, `{`, `}`, `(`, `)`, `|`,
    /// `^`, `$`, `\\`, whitespace).
    DisallowedChar,
    /// The pattern started with a dot, ended with a dot (not `.*`),
    /// or contained `..` — none are valid tool ids and none are valid
    /// prefixes.
    BadDots,
    /// The trailing `.*` form was present but the prefix before it was
    /// empty (e.g. `.*`). A glob with no namespace would match every
    /// tool the runner has ever registered; the substrate doc names
    /// this as a non-feature, not a footgun to ban with a runtime
    /// check.
    EmptyPrefix,
}

impl fmt::Display for AllowedToolPatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty allowed-tools pattern"),
            Self::StarOutsideTail => {
                f.write_str("allowed-tools pattern: `*` only allowed as a trailing `.*` suffix")
            }
            Self::DisallowedChar => {
                f.write_str("allowed-tools pattern: regex/glob metacharacters not allowed")
            }
            Self::BadDots => f.write_str(
                "allowed-tools pattern: leading dot, trailing dot, or `..` is not a tool id",
            ),
            Self::EmptyPrefix => {
                f.write_str("allowed-tools pattern: `.*` without a prefix matches everything")
            }
        }
    }
}

impl std::error::Error for AllowedToolPatternError {}

/// Validate one entry from a persona's `allowed_tools` list. Returns
/// `Ok(())` for a legal literal id or `prefix.*` glob and the matching
/// `AllowedToolPatternError` otherwise. The manifest reader calls this
/// at plugin load; the runtime calls it when it ingests a persona row.
pub fn validate_pattern(pattern: &str) -> Result<(), AllowedToolPatternError> {
    if pattern.is_empty() {
        return Err(AllowedToolPatternError::Empty);
    }

    // Disallowed characters anywhere: anything that suggests a regex,
    // shell glob, or whitespace. The set is closed on purpose — a future
    // syntax extension reopens it here and only here.
    if pattern.chars().any(|c| {
        matches!(
            c,
            '?' | '[' | ']' | '{' | '}' | '(' | ')' | '|' | '^' | '$' | '\\'
        ) || c.is_whitespace()
    }) {
        return Err(AllowedToolPatternError::DisallowedChar);
    }

    if pattern.starts_with('.') || pattern.contains("..") {
        return Err(AllowedToolPatternError::BadDots);
    }

    // Split the optional `.*` tail off and check the prefix shape.
    let body = if let Some(prefix) = pattern.strip_suffix(".*") {
        if prefix.is_empty() {
            return Err(AllowedToolPatternError::EmptyPrefix);
        }
        prefix
    } else {
        if pattern.ends_with('.') {
            return Err(AllowedToolPatternError::BadDots);
        }
        pattern
    };

    // No `*` anywhere in the body — the trailing `.*` has already been
    // stripped, so any remaining star is illegal regardless of position.
    if body.contains('*') {
        return Err(AllowedToolPatternError::StarOutsideTail);
    }

    Ok(())
}

/// Does `tool_id` match `pattern`? The function does not call
/// `validate_pattern` — that is a one-shot check at load time and
/// putting it here would turn every hot-path lookup into a second
/// scan. Callers are expected to have validated already; a malformed
/// pattern reaching this function is a programmer error and returns
/// `false` (no match).
pub fn pattern_matches(pattern: &str, tool_id: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(".*") {
        if prefix.is_empty() {
            return false;
        }
        // The substrate-doc rule: tool id must start with the prefix
        // *and* the next character must be a dot. `estimate.*` matches
        // `estimate.new` but not `estimaterunner`.
        tool_id.len() > prefix.len()
            && tool_id.starts_with(prefix)
            && tool_id.as_bytes()[prefix.len()] == b'.'
    } else {
        pattern == tool_id
    }
}

/// Is `tool_id` allowed by *any* entry in `patterns`? Convenience
/// over `pattern_matches`. Returns `false` for an empty patterns list
/// — a persona with no allowed tools is the cap "this thread cannot
/// call any tools," not the inverse.
pub fn tool_allowed<S: AsRef<str>>(patterns: &[S], tool_id: &str) -> bool {
    patterns
        .iter()
        .any(|p| pattern_matches(p.as_ref(), tool_id))
}

/// Validate every entry in a persona's `allowed_tools` list, returning
/// the first malformed entry's index and error. The manifest reader
/// uses this to point the operator at the bad row in `plugin.toml`.
pub fn validate_patterns<S: AsRef<str>>(
    patterns: &[S],
) -> Result<(), (usize, AllowedToolPatternError)> {
    for (i, p) in patterns.iter().enumerate() {
        validate_pattern(p.as_ref()).map_err(|e| (i, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_pattern_matches_only_exact_id() {
        assert!(pattern_matches("fs.read", "fs.read"));
        assert!(!pattern_matches("fs.read", "fs.read_dir"));
        assert!(!pattern_matches("fs.read", "fs.reader"));
        assert!(!pattern_matches("fs.read", "Fs.read"));
        assert!(!pattern_matches("fs.read", "fs.write"));
    }

    #[test]
    fn prefix_glob_requires_dot_boundary() {
        assert!(pattern_matches("estimate.*", "estimate.new"));
        assert!(pattern_matches("estimate.*", "estimate.bom_compute"));
        assert!(pattern_matches("estimate.*", "estimate.deeply.nested.tool"));
        // No dot after the prefix is not a match — the suffix rule
        // requires the namespace boundary.
        assert!(!pattern_matches("estimate.*", "estimaterunner"));
        assert!(!pattern_matches("estimate.*", "estimate"));
    }

    #[test]
    fn tool_allowed_walks_pattern_list() {
        let patterns = ["fs.read".to_owned(), "estimate.*".to_owned()];
        assert!(tool_allowed(&patterns, "fs.read"));
        assert!(tool_allowed(&patterns, "estimate.new"));
        assert!(!tool_allowed(&patterns, "fs.write"));
        let empty: [&str; 0] = [];
        assert!(!tool_allowed(&empty, "fs.read"));
    }

    #[test]
    fn validator_accepts_literals_and_prefix_globs() {
        for p in ["fs.read", "estimate.new", "estimate.*", "a.b.c.*", "single"] {
            assert!(validate_pattern(p).is_ok(), "rejected legal pattern: {p}");
        }
    }

    #[test]
    fn validator_rejects_empty_and_bad_dots() {
        assert_eq!(validate_pattern(""), Err(AllowedToolPatternError::Empty));
        assert_eq!(
            validate_pattern(".fs.read"),
            Err(AllowedToolPatternError::BadDots)
        );
        assert_eq!(
            validate_pattern("fs.read."),
            Err(AllowedToolPatternError::BadDots)
        );
        assert_eq!(
            validate_pattern("fs..read"),
            Err(AllowedToolPatternError::BadDots)
        );
    }

    #[test]
    fn validator_rejects_stars_outside_tail() {
        assert_eq!(
            validate_pattern("*.read"),
            Err(AllowedToolPatternError::StarOutsideTail)
        );
        assert_eq!(
            validate_pattern("est*mate"),
            Err(AllowedToolPatternError::StarOutsideTail)
        );
        // `estimate.*.bom` — the trailing `.*` strip leaves
        // `estimate.*.bom` minus the trailing `.*`? Actually
        // `estimate.*.bom` does not end with `.*`. The remaining `*`
        // in the body fails the no-star rule.
        assert_eq!(
            validate_pattern("estimate.*.bom"),
            Err(AllowedToolPatternError::StarOutsideTail)
        );
    }

    #[test]
    fn validator_rejects_regex_glob_metachars() {
        for p in [
            "fs.(read|write)",
            "fs.[rw]ead",
            "fs.{read,write}",
            "fs.read?",
            "fs.read$",
            "fs\\.read",
            "fs.r ead",
        ] {
            let got = validate_pattern(p);
            assert_eq!(
                got,
                Err(AllowedToolPatternError::DisallowedChar),
                "expected DisallowedChar for {p:?}, got {got:?}",
            );
        }
    }

    #[test]
    fn validator_rejects_bare_star_glob() {
        assert_eq!(
            validate_pattern(".*"),
            // Leading-dot check fires first; either of these answers
            // signals "no" to the caller — the precedence is incidental
            // and we just pin it here so a reorder is a visible diff.
            Err(AllowedToolPatternError::BadDots),
        );
    }

    #[test]
    fn validate_patterns_returns_first_bad_index() {
        let patterns = ["fs.read", "estimate.*", "bad pattern", "fs.write"];
        let err = validate_patterns(&patterns).unwrap_err();
        assert_eq!(err.0, 2);
        assert_eq!(err.1, AllowedToolPatternError::DisallowedChar);
    }

    #[test]
    fn malformed_pattern_in_match_is_false_not_panic() {
        // Documented behaviour: pattern_matches does not re-validate;
        // a programmer error returns false rather than crashing the
        // runner mid-turn.
        assert!(!pattern_matches("", "fs.read"));
        assert!(!pattern_matches(".*", "fs.read"));
    }
}
