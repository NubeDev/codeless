//! REVIEW-stage verdict parser.
//!
//! A REVIEW stage is a blocking gate: the model runs against the
//! stage's prompt and writes a handover whose body must include a
//! single `PASS:` or `FAIL:` sentinel line. The runtime parses the
//! sentinel and turns it into a `StageStatus::Passed` (run continues)
//! or `StageStatus::Failed` (run halts) — there is no third
//! "indeterminate" outcome; a missing or ambiguous sentinel is a
//! protocol violation and treated as failure so a silent model never
//! quietly drops a job through the gate.
//!
//! Scope: this module is the **sentinel parser only**. It does not
//! emit `ScopePatch` proposals (Step 4) and does not enforce patch
//! shape (Step 5). Keeping the surface narrow now means later stages
//! extend, rather than redesign.

/// Verdict the model reported via the sentinel line. The variant
/// carries the reason text the model wrote after the colon so the
/// runtime can attach it to the stage-completed envelope and the
/// handover for downstream consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewVerdict {
    Pass { reason: String },
    Fail { reason: String },
}

/// Why the parser refused to translate text into a verdict. Each
/// variant is itself a gate failure — `template_runner` maps both to
/// `RunnerOutcome::Failed` — but kept distinct so log lines and
/// future telemetry can tell "the model forgot the sentinel" apart
/// from "the model wrote both."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictParseError {
    /// No `PASS:` or `FAIL:` line found anywhere in the body.
    Missing,
    /// More than one sentinel line found. The model must pick one
    /// verdict per REVIEW stage; emitting both is ambiguous.
    Multiple,
}

impl std::fmt::Display for VerdictParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerdictParseError::Missing => {
                f.write_str("REVIEW handover did not contain a `PASS:` or `FAIL:` sentinel line")
            }
            VerdictParseError::Multiple => f.write_str(
                "REVIEW handover contained multiple `PASS:` / `FAIL:` sentinel lines; \
                 the gate must report exactly one verdict",
            ),
        }
    }
}

impl std::error::Error for VerdictParseError {}

/// Parse a single `PASS:` or `FAIL:` verdict from a handover body.
///
/// The sentinel is a line whose first non-whitespace content is
/// `PASS:` or `FAIL:` (case-sensitive), followed by free-form reason
/// text. The line may live in any handover section; the parser scans
/// the whole body rather than locking the convention to a specific
/// heading so a future stage can move the line without breaking the
/// parser. A bullet prefix (`- PASS: …`, `* PASS: …`) is permitted
/// and stripped.
///
/// Exactly one sentinel must be present. Zero ⇒ `Missing`. Two or
/// more ⇒ `Multiple` (including a `PASS:` and a `FAIL:` together).
pub fn parse_review_verdict(body: &str) -> Result<ReviewVerdict, VerdictParseError> {
    let mut found: Option<ReviewVerdict> = None;
    for line in body.lines() {
        let trimmed = strip_bullet(line.trim_start());
        let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("PASS:") {
            ("PASS", rest)
        } else if let Some(rest) = trimmed.strip_prefix("FAIL:") {
            ("FAIL", rest)
        } else {
            continue;
        };
        let reason = rest.trim().to_string();
        let next = if kind == "PASS" {
            ReviewVerdict::Pass { reason }
        } else {
            ReviewVerdict::Fail { reason }
        };
        if found.is_some() {
            return Err(VerdictParseError::Multiple);
        }
        found = Some(next);
    }
    found.ok_or(VerdictParseError::Missing)
}

/// Strip a leading markdown bullet marker (`- `, `* `, `+ `) so the
/// sentinel can land inside a bulleted list without confusing the
/// parser. Whitespace-only stripping; the bullet character itself is
/// the only structural change.
fn strip_bullet(s: &str) -> &str {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return rest;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pass_with_reason() {
        let body = "Some prose.\n\nPASS: rule R5 holds; no new permissions added.\n";
        let v = parse_review_verdict(body).unwrap();
        assert_eq!(
            v,
            ReviewVerdict::Pass {
                reason: "rule R5 holds; no new permissions added.".to_string()
            }
        );
    }

    #[test]
    fn parses_fail_with_reason() {
        let body = "FAIL: WORK touched DOCS/JOB-MODEL.md, which is wire-format.\n";
        let v = parse_review_verdict(body).unwrap();
        assert_eq!(
            v,
            ReviewVerdict::Fail {
                reason: "WORK touched DOCS/JOB-MODEL.md, which is wire-format.".to_string()
            }
        );
    }

    #[test]
    fn accepts_bullet_prefix() {
        let body = "## Done\n\n- PASS: looks good\n- other note\n";
        let v = parse_review_verdict(body).unwrap();
        match v {
            ReviewVerdict::Pass { reason } => assert_eq!(reason, "looks good"),
            other => panic!("expected Pass, got {other:?}"),
        }
    }

    #[test]
    fn missing_sentinel_is_error() {
        let body = "## Done\n\n- did some things\n\n## Next\n\n- (none)\n";
        assert_eq!(parse_review_verdict(body), Err(VerdictParseError::Missing));
    }

    #[test]
    fn pass_and_fail_together_rejected() {
        let body = "PASS: a\nFAIL: b\n";
        assert_eq!(parse_review_verdict(body), Err(VerdictParseError::Multiple));
    }

    #[test]
    fn two_pass_lines_rejected() {
        let body = "PASS: first\nPASS: second\n";
        assert_eq!(parse_review_verdict(body), Err(VerdictParseError::Multiple));
    }

    #[test]
    fn case_sensitive_lowercase_pass_is_not_sentinel() {
        // The sentinel is uppercase by contract; a lowercase mention
        // ("the test passes") must not trip the parser.
        let body = "the test passes nicely\n";
        assert_eq!(parse_review_verdict(body), Err(VerdictParseError::Missing));
    }

    #[test]
    fn sentinel_inside_indented_block_is_ignored_when_prefix_is_not_recognised() {
        // A `>` blockquote prefix is not stripped; only bullets are. A
        // quoted PASS: should not be picked up as a verdict.
        let body = "> PASS: quoted from elsewhere\n\nFAIL: real verdict\n";
        let v = parse_review_verdict(body).unwrap();
        match v {
            ReviewVerdict::Fail { .. } => {}
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn empty_reason_is_preserved() {
        // The parser keeps the reason text the model wrote, even when
        // it's empty. The caller decides whether an empty reason is a
        // protocol failure — the parser's job is just to translate.
        let body = "PASS:\n";
        let v = parse_review_verdict(body).unwrap();
        assert_eq!(
            v,
            ReviewVerdict::Pass {
                reason: String::new()
            }
        );
    }
}
