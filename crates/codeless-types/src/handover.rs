use serde::{Deserialize, Serialize};

/// Session-handover contract — see `DOCS/JOB-MODEL.md` "the handover
/// is the only contract between sessions". A run writes one of these
/// at `runs/<job_id>/handover.md` in its worktree on completion; the
/// next session reads it first, before anything else, to decide what
/// to do next.
///
/// The four sections are deliberately load-bearing:
/// - `done` is what landed — committed code, decisions ratified by
///   review, anything the next session does *not* need to redo.
/// - `next` is what the next session should pick up first; the top
///   item is the canonical next action.
/// - `what_you_need_to_know` carries the constraints, invariants, and
///   gotchas a fresh reader would not infer from the diff alone.
/// - `open_questions` is for unresolved decisions — the next session
///   resolves these before it implements anything new.
///
/// A blank handover is forbidden (JOB-MODEL.md): at minimum the
/// "Done" section must say *something*, even if it's "session aborted
/// at <stage>, recovery needed".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Handover {
    pub done: Vec<String>,
    pub next: Vec<String>,
    pub what_you_need_to_know: Vec<String>,
    pub open_questions: Vec<String>,
}

impl Handover {
    /// Render to the JOB-MODEL.md markdown shape: four `##` headings
    /// in fixed order, each section a bullet list. Empty sections
    /// render their heading with a placeholder bullet so a downstream
    /// parser still finds the four expected sections (a missing
    /// heading from a partial run is observably wrong).
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        write_section(&mut out, "Done", &self.done);
        write_section(&mut out, "Next", &self.next);
        write_section(
            &mut out,
            "What you need to know",
            &self.what_you_need_to_know,
        );
        write_section(&mut out, "Open questions", &self.open_questions);
        out
    }

    /// Parse a handover document back into the structured form. Robust
    /// to extra prose between sections (it lands on the floor) and to
    /// minor heading variations (`#`/`##`/`###`); strict about the
    /// section names so a typo surfaces instead of silently dropping
    /// content. Unrecognised sections are also dropped — the contract
    /// is the four canonical sections, full stop.
    ///
    /// The placeholder bullet `(none)` round-trips to an empty `Vec`
    /// so a never-populated section does not look like a real entry.
    pub fn from_markdown(src: &str) -> Result<Self, HandoverParseError> {
        let mut handover = Handover::default();
        let mut current: Option<&'static str> = None;
        let mut buf: Vec<String> = Vec::new();
        for raw in src.lines() {
            let line = raw.trim_end();
            if let Some(heading) = strip_heading(line) {
                if let Some(name) = current.take() {
                    store_section(&mut handover, name, std::mem::take(&mut buf))?;
                }
                current = Some(
                    match_section_name(heading)
                        .ok_or_else(|| HandoverParseError::UnknownSection(heading.to_string()))?,
                );
                continue;
            }
            if current.is_none() {
                continue;
            }
            if let Some(item) = strip_bullet(line) {
                if !item.is_empty() && item != PLACEHOLDER {
                    buf.push(item.to_string());
                }
            }
        }
        if let Some(name) = current.take() {
            store_section(&mut handover, name, buf)?;
        }
        Ok(handover)
    }
}

fn write_section(out: &mut String, title: &str, items: &[String]) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str("## ");
    out.push_str(title);
    out.push_str("\n\n");
    if items.is_empty() {
        out.push_str("- ");
        out.push_str(PLACEHOLDER);
        out.push('\n');
        return;
    }
    for item in items {
        out.push_str("- ");
        // Multi-line bullets get continuation indentation so a
        // downstream parser keeps them with their parent item rather
        // than treating the second line as a new top-level entry. The
        // model is encouraged to keep bullets single-line in the
        // first place; this is only here so we do not corrupt content
        // that arrives wrapped.
        let mut lines = item.split('\n');
        if let Some(first) = lines.next() {
            out.push_str(first);
            out.push('\n');
        }
        for cont in lines {
            out.push_str("  ");
            out.push_str(cont);
            out.push('\n');
        }
    }
}

fn strip_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("###").or_else(|| {
        trimmed
            .strip_prefix("##")
            .or_else(|| trimmed.strip_prefix('#'))
    })?;
    let body = rest.trim();
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

fn strip_bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?;
    Some(rest.trim())
}

fn match_section_name(heading: &str) -> Option<&'static str> {
    let normalised = heading.trim().to_ascii_lowercase();
    match normalised.as_str() {
        "done" => Some("done"),
        "next" => Some("next"),
        "what you need to know" => Some("what_you_need_to_know"),
        "open questions" => Some("open_questions"),
        _ => None,
    }
}

fn store_section(
    handover: &mut Handover,
    name: &'static str,
    items: Vec<String>,
) -> Result<(), HandoverParseError> {
    let target = match name {
        "done" => &mut handover.done,
        "next" => &mut handover.next,
        "what_you_need_to_know" => &mut handover.what_you_need_to_know,
        "open_questions" => &mut handover.open_questions,
        other => return Err(HandoverParseError::UnknownSection(other.to_string())),
    };
    *target = items;
    Ok(())
}

const PLACEHOLDER: &str = "(none)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverParseError {
    UnknownSection(String),
}

impl std::fmt::Display for HandoverParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoverParseError::UnknownSection(s) => {
                write!(f, "unknown handover section heading: {s:?}")
            }
        }
    }
}

impl std::error::Error for HandoverParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_populated() {
        let h = Handover {
            done: vec!["wrote stage 1".into(), "fixed verify".into()],
            next: vec!["land stage 2".into()],
            what_you_need_to_know: vec!["caps are in cents".into()],
            open_questions: vec!["should we ship?".into()],
        };
        let parsed = Handover::from_markdown(&h.to_markdown()).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn roundtrip_empty_sections_render_placeholder_and_parse_back_to_empty() {
        let h = Handover::default();
        let md = h.to_markdown();
        assert!(md.contains("## Done"));
        assert!(md.contains("## Next"));
        assert!(md.contains("## What you need to know"));
        assert!(md.contains("## Open questions"));
        assert!(md.contains("- (none)"));
        let parsed = Handover::from_markdown(&md).unwrap();
        assert_eq!(parsed, Handover::default());
    }

    #[test]
    fn section_ordering_is_stable() {
        let h = Handover {
            done: vec!["a".into()],
            next: vec!["b".into()],
            what_you_need_to_know: vec!["c".into()],
            open_questions: vec!["d".into()],
        };
        let md = h.to_markdown();
        let i_done = md.find("## Done").unwrap();
        let i_next = md.find("## Next").unwrap();
        let i_wynt = md.find("## What you need to know").unwrap();
        let i_open = md.find("## Open questions").unwrap();
        assert!(i_done < i_next && i_next < i_wynt && i_wynt < i_open);
    }

    #[test]
    fn unknown_section_errors() {
        let bad = "## Sidebar\n- nope\n";
        match Handover::from_markdown(bad) {
            Err(HandoverParseError::UnknownSection(s)) => assert_eq!(s, "Sidebar"),
            other => panic!("expected unknown-section error, got {other:?}"),
        }
    }

    #[test]
    fn prose_between_bullets_is_ignored() {
        let src = "## Done\n\nthis is some prose the model emitted\n- the real bullet\n";
        let parsed = Handover::from_markdown(src).unwrap();
        assert_eq!(parsed.done, vec!["the real bullet".to_string()]);
    }

    #[test]
    fn asterisk_bullets_accepted() {
        let src = "## Done\n* one\n* two\n";
        let parsed = Handover::from_markdown(src).unwrap();
        assert_eq!(parsed.done, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn case_insensitive_section_names() {
        let src = "## DONE\n- yes\n## next\n- soon\n";
        let parsed = Handover::from_markdown(src).unwrap();
        assert_eq!(parsed.done, vec!["yes".to_string()]);
        assert_eq!(parsed.next, vec!["soon".to_string()]);
    }
}
