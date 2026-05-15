//! Read side of `DOCS/SCOPE-PROPOSED.md`, paired with the writer in
//! [`scope_patch_emit`]. Step 6 of the SESSION-MUTABLE-SCOPE ramp
//! consumes this: the `codeless patches` CLI parses the proposals
//! file via [`parse_queue`], shows individual entries via
//! [`Proposal::render`], removes an approved or rejected entry via
//! [`remove_entry`], and rewrites the file via [`write_queue`].
//!
//! The format is the inverse of `render_proposal_markdown` in
//! `scope_patch_emit`: a top-of-file header followed by zero or more
//! `## <ulid>` blocks, each carrying a bulleted metadata header and
//! `### Rationale` / `### Body` sections. The parser is deliberately
//! tolerant — it ignores keys it does not recognise and only fails
//! when the per-block id at the heading is not a parseable
//! `ScopePatchId`, because a `## <non-ulid>` line is almost certainly
//! free-form prose the operator added and not a proposal at all.

use std::path::Path;
use std::str::FromStr;

use codeless_types::{ScopePatchId, ScopePatchKind, ScopePatchTarget, StageId};

/// One proposal as read from `DOCS/SCOPE-PROPOSED.md`. Mirrors the
/// fields `render_proposal_markdown` writes; the parser-only fields
/// `predicate_ref` / `fixture_ref` are preserved here because Step 6's
/// approval CLI shows them to the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: ScopePatchId,
    pub kind: ScopePatchKind,
    pub target: ScopePatchTarget,
    pub target_path: String,
    pub rationale: String,
    pub body: String,
    pub has_predicate: bool,
    pub evidence_stage_id: Option<StageId>,
    pub predicate_ref: Option<String>,
    pub fixture_ref: Option<String>,
    /// The exact span of the file the proposal occupies, from the
    /// leading blank line before its `## <id>` heading through to the
    /// last line of its body. [`remove_entry`] uses this to slice the
    /// proposal out without disturbing surrounding entries or the
    /// file header. Stored as character offsets so a non-ASCII
    /// rationale or body does not corrupt the slice.
    pub span: (usize, usize),
}

impl Proposal {
    /// Re-render this proposal as the markdown chunk that would
    /// appear in `DOCS/SCOPE-PROPOSED.md`. Used by `codeless patches
    /// show` so the human reads the same block the runtime wrote,
    /// not a re-formatted summary. Output starts with the `## <id>`
    /// heading (no leading blank line); the caller adds spacing as
    /// needed.
    pub fn render(&self) -> String {
        let kind = match self.kind {
            ScopePatchKind::Tighten => "tighten",
            ScopePatchKind::Loosen => "loosen",
        };
        let target = match self.target {
            ScopePatchTarget::ClaudeMd => "claude-md",
            ScopePatchTarget::JobScopeMd => "job-scope-md",
            ScopePatchTarget::JobWorkflowMd => "job-workflow-md",
            ScopePatchTarget::JobClaudeMd => "job-claude-md",
        };
        let mut out = String::new();
        out.push_str("## ");
        out.push_str(&self.id.to_string());
        out.push_str("\n\n");
        out.push_str(&format!(
            "- kind: {kind}\n- target: {target}\n- target-path: {}\n- has_predicate: {}\n",
            self.target_path, self.has_predicate,
        ));
        if let Some(ev) = self.evidence_stage_id {
            out.push_str(&format!("- evidence_stage_id: {ev}\n"));
        }
        if let Some(pr) = &self.predicate_ref {
            out.push_str(&format!("- predicate-ref: {pr}\n"));
        }
        if let Some(fx) = &self.fixture_ref {
            out.push_str(&format!("- fixture-ref: {fx}\n"));
        }
        out.push_str("\n### Rationale\n\n");
        out.push_str(self.rationale.trim_end());
        out.push_str("\n\n### Body\n\n");
        out.push_str(self.body.trim_end());
        out.push('\n');
        out
    }
}

/// Parsed proposals from `DOCS/SCOPE-PROPOSED.md`, plus the leading
/// header text (everything before the first `## <ulid>` heading). The
/// header is preserved on rewrite so `codeless patches approve`
/// removing the last entry does not strip the "queue of proposals"
/// preamble.
#[derive(Debug, Default)]
pub struct Queue {
    pub header: String,
    pub proposals: Vec<Proposal>,
}

impl Queue {
    /// Render the queue back to the on-disk markdown form. Identity
    /// holds for a queue parsed from a file written by
    /// `scope_patch_emit::render_proposal_markdown`: the round-trip
    /// preserves byte content modulo the trim_end normalisation in
    /// `Proposal::render` (which strips trailing whitespace from
    /// rationale/body — same shape `scope_patch_emit` emits).
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.header);
        for p in &self.proposals {
            if !out.ends_with("\n\n") {
                if out.ends_with('\n') {
                    out.push('\n');
                } else if !out.is_empty() {
                    out.push_str("\n\n");
                }
            }
            out.push_str(&p.render());
        }
        out
    }

    /// Find a proposal by id. Returns `None` if the queue does not
    /// contain it; the CLI converts that into a non-zero exit code
    /// with a "no such patch" message.
    pub fn find(&self, id: ScopePatchId) -> Option<&Proposal> {
        self.proposals.iter().find(|p| p.id == id)
    }

    /// Remove a proposal by id, returning the removed record so the
    /// caller can cite it in the approval commit body. Returns `None`
    /// if the id is not present.
    pub fn remove(&mut self, id: ScopePatchId) -> Option<Proposal> {
        let idx = self.proposals.iter().position(|p| p.id == id)?;
        Some(self.proposals.remove(idx))
    }

    /// Replace a proposal in-place. Used by `codeless patches edit`
    /// after the operator's editor returns. Returns `true` if the id
    /// matched an existing entry and was replaced, `false` if no such
    /// id is in the queue.
    pub fn replace(&mut self, edited: Proposal) -> bool {
        let id = edited.id;
        let Some(idx) = self.proposals.iter().position(|p| p.id == id) else {
            return false;
        };
        self.proposals[idx] = edited;
        true
    }
}

/// What can go wrong loading the queue from disk. The CLI maps each
/// variant onto a distinct error message — `Io` and `Missing` are
/// indistinguishable to a human ("you have not run a job that emits
/// a patch yet, or the file is gone"), but `Parse` is loud because
/// a malformed proposals file means the runtime wrote something the
/// reader does not understand and that is a bug worth surfacing.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("DOCS/SCOPE-PROPOSED.md not found at {path}")]
    Missing { path: String },
    #[error("read DOCS/SCOPE-PROPOSED.md ({path}): {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse DOCS/SCOPE-PROPOSED.md: {0}")]
    Parse(String),
}

/// Read and parse the queue from a worktree root. Returns
/// [`QueueError::Missing`] when the file does not exist so the CLI
/// can print "no proposed patches" without an error exit.
pub fn load_queue(worktree: &Path) -> Result<Queue, QueueError> {
    let path = worktree.join("DOCS").join("SCOPE-PROPOSED.md");
    let display = path.display().to_string();
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(QueueError::Missing { path: display });
        }
        Err(source) => {
            return Err(QueueError::Io {
                path: display,
                source,
            })
        }
    };
    parse_queue(&body).map_err(QueueError::Parse)
}

/// Write the queue back to disk. Idempotent in the sense that
/// `write_queue(load_queue(_)?)` is a no-op when the parser preserved
/// every byte.
pub fn write_queue(worktree: &Path, queue: &Queue) -> Result<(), QueueError> {
    let docs = worktree.join("DOCS");
    if let Err(source) = std::fs::create_dir_all(&docs) {
        return Err(QueueError::Io {
            path: docs.display().to_string(),
            source,
        });
    }
    let path = docs.join("SCOPE-PROPOSED.md");
    let display = path.display().to_string();
    std::fs::write(&path, queue.to_markdown()).map_err(|source| QueueError::Io {
        path: display,
        source,
    })
}

/// Parse the markdown form of the proposals file. Public for the
/// CLI's edit flow, which re-parses the operator's edited buffer
/// before writing it back.
pub fn parse_queue(body: &str) -> Result<Queue, String> {
    let blocks = split_blocks(body);
    let header = blocks.header.to_string();
    let mut proposals = Vec::with_capacity(blocks.blocks.len());
    for block in blocks.blocks {
        proposals.push(parse_block(block.text, block.span)?);
    }
    Ok(Queue { header, proposals })
}

struct BlockRef<'a> {
    text: &'a str,
    span: (usize, usize),
}

struct SplitResult<'a> {
    header: &'a str,
    blocks: Vec<BlockRef<'a>>,
}

/// Split the file into `(header, [block])` chunks. A "block" is the
/// region from a `## <ulid>` heading (with its preceding blank line,
/// if any) through to the byte before the next such heading.
fn split_blocks(body: &str) -> SplitResult<'_> {
    let mut starts: Vec<usize> = Vec::new();
    for (i, line) in line_offsets(body) {
        if let Some(rest) = line.strip_prefix("## ") {
            let id_token = rest.trim();
            if ScopePatchId::from_str(id_token).is_ok() {
                let block_start = if i > 0 && body.as_bytes().get(i - 1) == Some(&b'\n') {
                    let prev = body[..i - 1].rfind('\n').map(|n| n + 1).unwrap_or(0);
                    if body[prev..i - 1].trim().is_empty() {
                        prev
                    } else {
                        i
                    }
                } else {
                    i
                };
                starts.push(block_start);
            }
        }
    }
    if starts.is_empty() {
        return SplitResult {
            header: body,
            blocks: Vec::new(),
        };
    }
    let header_end = starts[0];
    let mut blocks = Vec::with_capacity(starts.len());
    for (idx, &start) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).copied().unwrap_or(body.len());
        blocks.push(BlockRef {
            text: &body[start..end],
            span: (start, end),
        });
    }
    SplitResult {
        header: &body[..header_end],
        blocks,
    }
}

/// Iterate over `(byte_offset, line_text)`. Lines exclude the
/// trailing `\n`. We need offsets, not just lines, so the parser can
/// reconstruct the byte span of each block.
fn line_offsets(body: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= body.len() {
            return None;
        }
        let rel = body[start..].find('\n');
        let (line, advance) = match rel {
            Some(n) => (&body[start..start + n], n + 1),
            None => (&body[start..], body.len() - start),
        };
        let here = start;
        start += advance;
        Some((here, line))
    })
}

/// Parse one block's text. The heading is `## <id>`; everything else
/// is bulleted metadata and `### Rationale` / `### Body` sections.
fn parse_block(text: &str, span: (usize, usize)) -> Result<Proposal, String> {
    let mut lines = text.lines().peekable();
    while let Some(l) = lines.peek() {
        if l.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    let heading = lines
        .next()
        .ok_or_else(|| "empty proposal block".to_string())?;
    let id_str = heading
        .strip_prefix("## ")
        .ok_or_else(|| format!("block heading is not `## <id>`: {heading}"))?
        .trim();
    let id = ScopePatchId::from_str(id_str)
        .map_err(|e| format!("block heading id `{id_str}` is not a ScopePatchId: {e}"))?;

    let mut kind: Option<ScopePatchKind> = None;
    let mut target: Option<ScopePatchTarget> = None;
    let mut target_path: Option<String> = None;
    let mut has_predicate = false;
    let mut evidence_stage_id: Option<StageId> = None;
    let mut predicate_ref: Option<String> = None;
    let mut fixture_ref: Option<String> = None;
    let mut section: Section = Section::Meta;
    let mut rationale = String::new();
    let mut body = String::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "### Rationale" {
            section = Section::Rationale;
            continue;
        }
        if trimmed == "### Body" {
            section = Section::Body;
            continue;
        }
        match section {
            Section::Meta => {
                let Some(rest) = trimmed.strip_prefix("- ") else {
                    continue;
                };
                let Some((key, value)) = rest.split_once(':') else {
                    continue;
                };
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim();
                match key.as_str() {
                    "kind" => {
                        kind = Some(match value.to_ascii_lowercase().as_str() {
                            "tighten" => ScopePatchKind::Tighten,
                            "loosen" => ScopePatchKind::Loosen,
                            other => return Err(format!("unknown kind `{other}` in block {id}")),
                        });
                    }
                    "target" => {
                        target = Some(match value.to_ascii_lowercase().as_str() {
                            "claude-md" => ScopePatchTarget::ClaudeMd,
                            "job-scope-md" => ScopePatchTarget::JobScopeMd,
                            "job-workflow-md" => ScopePatchTarget::JobWorkflowMd,
                            "job-claude-md" => ScopePatchTarget::JobClaudeMd,
                            other => return Err(format!("unknown target `{other}` in block {id}")),
                        });
                    }
                    "target-path" | "target_path" => {
                        target_path = Some(value.to_string());
                    }
                    "has_predicate" | "has-predicate" | "predicate" => {
                        has_predicate =
                            matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "1");
                    }
                    "evidence_stage_id" | "evidence-stage-id" | "evidence" => {
                        evidence_stage_id = Some(StageId::from_str(value).map_err(|e| {
                            format!("evidence_stage_id in block {id} is not a ULID: {e}")
                        })?);
                    }
                    "predicate-ref" | "predicate_ref" => {
                        predicate_ref = Some(value.to_string());
                    }
                    "fixture-ref" | "fixture_ref" => {
                        fixture_ref = Some(value.to_string());
                    }
                    _ => {}
                }
            }
            Section::Rationale => {
                if !rationale.is_empty() {
                    rationale.push('\n');
                }
                rationale.push_str(line);
            }
            Section::Body => {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(line);
            }
        }
    }

    let kind = kind.ok_or_else(|| format!("missing `kind` in block {id}"))?;
    let target = target.ok_or_else(|| format!("missing `target` in block {id}"))?;
    let target_path = target_path.ok_or_else(|| format!("missing `target-path` in block {id}"))?;

    Ok(Proposal {
        id,
        kind,
        target,
        target_path,
        rationale: rationale.trim().to_string(),
        body: body.trim_end().trim_start_matches('\n').to_string(),
        has_predicate,
        evidence_stage_id,
        predicate_ref,
        fixture_ref,
        span,
    })
}

enum Section {
    Meta,
    Rationale,
    Body,
}

/// Convenience: remove one proposal by id, write the file back, and
/// return the removed record. Used by `codeless patches approve`
/// and `codeless patches reject` — the only difference between the
/// two is the commit subject the CLI assembles afterwards.
pub fn remove_entry(worktree: &Path, id: ScopePatchId) -> Result<Proposal, QueueError> {
    let mut queue = load_queue(worktree)?;
    let removed = queue
        .remove(id)
        .ok_or_else(|| QueueError::Parse(format!("no proposed patch with id `{id}` in queue")))?;
    write_queue(worktree, &queue)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{ReviewId, ScopePatch};

    fn make_patch(kind: ScopePatchKind, target: ScopePatchTarget, target_path: &str) -> ScopePatch {
        ScopePatch {
            id: ScopePatchId::new(),
            review_id: ReviewId::new(),
            stage_id: StageId::new(),
            kind,
            target,
            target_path: target_path.into(),
            rationale: "because R4 should auto-FAIL".into(),
            body: "append a sentence to R4".into(),
            has_predicate: kind == ScopePatchKind::Tighten,
            evidence_stage_id: if kind == ScopePatchKind::Loosen {
                Some(StageId::new())
            } else {
                None
            },
        }
    }

    /// Build a proposals-file body the way `scope_patch_emit` would,
    /// without taking a dependency on that module's private writer.
    /// The shape matches `render_proposal_markdown` exactly so the
    /// parser/writer round-trips against the format the runtime
    /// actually emits.
    fn write_block(
        out: &mut String,
        p: &ScopePatch,
        predicate_ref: Option<&str>,
        fixture_ref: Option<&str>,
    ) {
        let kind = match p.kind {
            ScopePatchKind::Tighten => "tighten",
            ScopePatchKind::Loosen => "loosen",
        };
        let target = match p.target {
            ScopePatchTarget::ClaudeMd => "claude-md",
            ScopePatchTarget::JobScopeMd => "job-scope-md",
            ScopePatchTarget::JobWorkflowMd => "job-workflow-md",
            ScopePatchTarget::JobClaudeMd => "job-claude-md",
        };
        out.push_str("\n## ");
        out.push_str(&p.id.to_string());
        out.push_str("\n\n");
        out.push_str(&format!(
            "- kind: {kind}\n- target: {target}\n- target-path: {}\n- review_id: {}\n- stage_id: {}\n- has_predicate: {}\n",
            p.target_path, p.review_id, p.stage_id, p.has_predicate,
        ));
        if let Some(ev) = p.evidence_stage_id {
            out.push_str(&format!("- evidence_stage_id: {ev}\n"));
        }
        if let Some(pr) = predicate_ref {
            out.push_str(&format!("- predicate-ref: {pr}\n"));
        }
        if let Some(fx) = fixture_ref {
            out.push_str(&format!("- fixture-ref: {fx}\n"));
        }
        out.push_str("\n### Rationale\n\n");
        out.push_str(&p.rationale);
        out.push_str("\n\n### Body\n\n");
        out.push_str(&p.body);
        out.push('\n');
    }

    fn make_file(patches: &[(&ScopePatch, Option<&str>, Option<&str>)]) -> String {
        let mut out = String::from(
            "# Proposed scope patches\n\nThis file is the queue of REVIEW-emitted patches.\n",
        );
        for (p, pr, fx) in patches {
            write_block(&mut out, p, *pr, *fx);
        }
        out
    }

    #[test]
    fn parse_empty_queue_yields_just_header() {
        let body = "# Proposed scope patches\n\nempty for now.\n";
        let q = parse_queue(body).unwrap();
        assert!(q.proposals.is_empty());
        assert_eq!(q.header, body);
    }

    #[test]
    fn parse_one_tighten_round_trips() {
        let p = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let body = make_file(&[(&p, Some("no-emojis-in-source"), None)]);
        let q = parse_queue(&body).unwrap();
        assert_eq!(q.proposals.len(), 1);
        let parsed = &q.proposals[0];
        assert_eq!(parsed.id, p.id);
        assert_eq!(parsed.kind, ScopePatchKind::Tighten);
        assert_eq!(parsed.target, ScopePatchTarget::ClaudeMd);
        assert_eq!(parsed.target_path, "codeless/CLAUDE.md");
        assert!(parsed.has_predicate);
        assert_eq!(parsed.predicate_ref.as_deref(), Some("no-emojis-in-source"));
        assert!(parsed.fixture_ref.is_none());
        assert_eq!(parsed.rationale, p.rationale);
        assert_eq!(parsed.body, p.body);
    }

    #[test]
    fn parse_loosen_carries_evidence_and_fixture() {
        let p = make_patch(
            ScopePatchKind::Loosen,
            ScopePatchTarget::JobScopeMd,
            ".codeless/jobs/x/SCOPE.md",
        );
        let body = make_file(&[(&p, None, Some("crates/codeless-predicates/tests/fix.rs"))]);
        let q = parse_queue(&body).unwrap();
        let parsed = &q.proposals[0];
        assert_eq!(parsed.kind, ScopePatchKind::Loosen);
        assert!(parsed.evidence_stage_id.is_some());
        assert_eq!(
            parsed.fixture_ref.as_deref(),
            Some("crates/codeless-predicates/tests/fix.rs")
        );
        assert!(!parsed.has_predicate);
    }

    #[test]
    fn parse_two_blocks_yields_two_proposals_with_distinct_spans() {
        let p1 = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let p2 = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::JobClaudeMd,
            ".codeless/jobs/x/CLAUDE.md",
        );
        let body = make_file(&[
            (&p1, Some("no-emojis-in-source"), None),
            (&p2, Some("no-emojis-in-source"), None),
        ]);
        let q = parse_queue(&body).unwrap();
        assert_eq!(q.proposals.len(), 2);
        assert_ne!(q.proposals[0].span, q.proposals[1].span);
        assert!(q.proposals[0].span.1 <= q.proposals[1].span.0);
    }

    #[test]
    fn remove_drops_only_the_named_entry_and_preserves_header() {
        let p1 = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let p2 = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::JobClaudeMd,
            ".codeless/jobs/x/CLAUDE.md",
        );
        let body = make_file(&[
            (&p1, Some("no-emojis-in-source"), None),
            (&p2, Some("no-emojis-in-source"), None),
        ]);
        let mut q = parse_queue(&body).unwrap();
        let removed = q.remove(p1.id).expect("first patch present");
        assert_eq!(removed.id, p1.id);
        assert_eq!(q.proposals.len(), 1);
        assert_eq!(q.proposals[0].id, p2.id);
        let rewritten = q.to_markdown();
        assert!(rewritten.contains("# Proposed scope patches"));
        assert!(!rewritten.contains(&p1.id.to_string()));
        assert!(rewritten.contains(&p2.id.to_string()));
    }

    #[test]
    fn load_missing_returns_missing() {
        let tmp = tempfile::tempdir().unwrap();
        match load_queue(tmp.path()) {
            Err(QueueError::Missing { .. }) => {}
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn remove_entry_unknown_id_returns_parse_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let body = make_file(&[(&p, Some("no-emojis-in-source"), None)]);
        std::fs::create_dir_all(tmp.path().join("DOCS")).unwrap();
        std::fs::write(tmp.path().join("DOCS/SCOPE-PROPOSED.md"), body).unwrap();
        let unknown = ScopePatchId::new();
        match remove_entry(tmp.path(), unknown) {
            Err(QueueError::Parse(msg)) => assert!(msg.contains("no proposed patch")),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_preserves_header_and_block_count() {
        let p1 = make_patch(
            ScopePatchKind::Loosen,
            ScopePatchTarget::JobWorkflowMd,
            ".codeless/jobs/x/WORKFLOW.md",
        );
        let p2 = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let body = make_file(&[
            (&p1, None, Some("crates/codeless-predicates/tests/y.rs")),
            (&p2, Some("no-process-spawn-outside-adapters-host"), None),
        ]);
        let q = parse_queue(&body).unwrap();
        let rewritten = q.to_markdown();
        let q2 = parse_queue(&rewritten).unwrap();
        assert_eq!(q2.proposals.len(), 2);
        assert_eq!(q2.proposals[0].id, p1.id);
        assert_eq!(q2.proposals[1].id, p2.id);
        assert_eq!(q2.proposals[0].fixture_ref, q.proposals[0].fixture_ref);
        assert_eq!(q2.proposals[1].predicate_ref, q.proposals[1].predicate_ref);
    }

    #[test]
    fn parse_block_without_kind_errors() {
        let id = ScopePatchId::new();
        let body = format!(
            "# header\n\n## {id}\n\n- target: claude-md\n- target-path: codeless/CLAUDE.md\n\n### Rationale\n\nr\n\n### Body\n\nb\n"
        );
        match parse_queue(&body) {
            Err(e) => assert!(e.contains("kind")),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn non_ulid_heading_does_not_split_a_block() {
        // A `## Notes` heading buried in a rationale section must
        // not be mistaken for a new proposal.
        let p = make_patch(
            ScopePatchKind::Tighten,
            ScopePatchTarget::ClaudeMd,
            "codeless/CLAUDE.md",
        );
        let body = format!(
            "# Proposed scope patches\n\n## {}\n\n- kind: tighten\n- target: claude-md\n- target-path: codeless/CLAUDE.md\n- has_predicate: true\n\n### Rationale\n\nsee discussion under ## Notes for context\n\n### Body\n\nappend the sentence\n",
            p.id
        );
        let q = parse_queue(&body).unwrap();
        assert_eq!(q.proposals.len(), 1);
        assert!(q.proposals[0].rationale.contains("## Notes"));
    }
}
