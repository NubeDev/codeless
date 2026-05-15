//! SESSION-MUTABLE-SCOPE Step 4 — shadow-mode emission of REVIEW-stage
//! `ScopePatch` proposals.
//!
//! A REVIEW stage that wants to suggest a rulebook change appends a
//! `SCOPE-PATCH-BEGIN` … `SCOPE-PATCH-END` block to its handover body
//! alongside the standard `PASS:` sentinel. After the gate parses
//! `PASS`, `template_runner` hands the same body to this module: if
//! exactly one well-formed block is present, the runtime allocates a
//! `ScopePatchId`, appends a human-readable record to
//! `DOCS/SCOPE-PROPOSED.md`, and publishes a `ScopePatchProposed`
//! envelope on the event bus.
//!
//! "Shadow mode" is what the ramp doc calls this stage — nothing
//! merges automatically. The proposal accumulates as a file artifact
//! (the Step 6 approval CLI will walk that file) and the event
//! envelope gives the kill-criterion query a data source per
//! `SESSION-MUTABLE-SCOPE-DECISIONS.md` Q7. Step 5 layers the
//! parse-time guards (one-per-REVIEW, mutable-set membership,
//! evidence requirements) on top of this floor; Step 4 deliberately
//! accepts loose proposals so a partial block still surfaces as a
//! visible event rather than silently dropping.
//!
//! Why a custom mini-format and not (say) JSON: the handover is
//! authored by a model into a markdown document a human will read. A
//! plain key:value block sits naturally next to the `PASS:` sentinel,
//! and the parser stays small enough to live in one file. Step 5 may
//! add a stricter validator, but the wire-shape on disk does not
//! change.

use std::path::Path;

use codeless_types::{
    Event, JobId, ReviewId, ScopePatch, ScopePatchId, ScopePatchKind, ScopePatchTarget, StageId,
};
use std::str::FromStr;

use crate::event_bus::EventBus;
use crate::time::now_ms;

/// Outcome of trying to extract a patch proposal from a REVIEW
/// handover body. The caller treats all variants except `Emitted` as
/// "no patch this stage" and continues — none of them are failures
/// in shadow mode.
#[derive(Debug)]
pub enum EmitOutcome {
    /// A well-formed proposal was parsed, written to
    /// `DOCS/SCOPE-PROPOSED.md`, and published as an event.
    Emitted(ScopePatchId),
    /// No `SCOPE-PATCH-BEGIN` / `END` markers in the body. The most
    /// common case — most REVIEW stages will pass without proposing
    /// a rulebook change.
    NoBlock,
    /// More than one block. Step 5 will turn this into a parse-time
    /// reject; Step 4 logs it and emits nothing so the kill-criterion
    /// telemetry does not double-count.
    MultipleBlocks,
    /// The block existed but parsing failed (missing required key,
    /// unknown `kind` / `target`, etc.). Logged with the reason; no
    /// event is emitted. Step 5 turns this into a FAIL verdict.
    Malformed(String),
    /// Parsing succeeded but a downstream side-effect (writing the
    /// proposals file, publishing the event) failed. Carried through
    /// so the caller logs at warn level; shadow-mode policy is to
    /// continue the job rather than turn an I/O wobble into a stage
    /// failure.
    SideEffectFailed(String),
}

/// Parse, persist, and publish in one shot. Called by
/// `template_runner` after a REVIEW gate returns `Pass`; safe to call
/// with a non-REVIEW handover body (will return `NoBlock` and do
/// nothing). The function never returns `Err`: shadow mode treats
/// every failure mode as observable-but-non-fatal, mapped onto the
/// variants above for the caller's structured log.
pub async fn emit_from_handover(
    bus: &EventBus,
    worktree: &Path,
    job_id: JobId,
    stage_id: StageId,
    review_id: ReviewId,
    handover_body: &str,
) -> EmitOutcome {
    let parsed = match parse_blocks(handover_body) {
        ParseResult::None => return EmitOutcome::NoBlock,
        ParseResult::Multiple => return EmitOutcome::MultipleBlocks,
        ParseResult::Malformed(reason) => return EmitOutcome::Malformed(reason),
        ParseResult::One(p) => p,
    };

    let patch_id = ScopePatchId::new();
    let patch = ScopePatch {
        id: patch_id,
        review_id,
        stage_id,
        kind: parsed.kind,
        target: parsed.target,
        target_path: parsed.target_path.clone(),
        rationale: parsed.rationale.clone(),
        body: parsed.body.clone(),
        has_predicate: parsed.has_predicate,
        evidence_stage_id: parsed.evidence_stage_id,
    };

    if let Err(err) = append_to_proposals_file(worktree, &patch).await {
        return EmitOutcome::SideEffectFailed(format!("append to DOCS/SCOPE-PROPOSED.md: {err}"));
    }

    let publish_result = bus
        .publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::ScopePatchProposed {
                stage_id,
                review_id,
                patch_id,
                kind: patch.kind,
                target: patch.target,
                target_path: patch.target_path.clone(),
                evidence_stage_id: patch.evidence_stage_id,
                has_predicate: patch.has_predicate,
            },
            now_ms(),
        )
        .await;
    if let Err(err) = publish_result {
        return EmitOutcome::SideEffectFailed(format!("publish ScopePatchProposed: {err}"));
    }

    EmitOutcome::Emitted(patch_id)
}

/// Append a human-readable record of the proposal to
/// `DOCS/SCOPE-PROPOSED.md`. The file is created if absent. The
/// format is deliberately append-only and lightly structured: a
/// `## <patch_id>` heading plus a fenced block carrying the
/// serialised proposal body so a future approval CLI can read it
/// back without re-parsing the prose. Decisions Q1 records why this
/// file is opaque to WORK stages.
async fn append_to_proposals_file(worktree: &Path, patch: &ScopePatch) -> std::io::Result<()> {
    let docs_dir = worktree.join("DOCS");
    tokio::fs::create_dir_all(&docs_dir).await?;
    let path = docs_dir.join("SCOPE-PROPOSED.md");

    let mut record = String::new();
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        record.push_str(
            "# Proposed scope patches\n\n\
             This file is the queue of REVIEW-emitted `ScopePatch` proposals \
             waiting for human approval. The runtime appends to it; the Step 6 \
             approval CLI is the only consumer that mutates entries. WORK stages \
             treat it as opaque (decisions Q1).\n",
        );
    }
    record.push_str(&render_proposal_markdown(patch));

    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    f.write_all(record.as_bytes()).await?;
    f.flush().await?;
    Ok(())
}

/// Render one proposal as the human-readable markdown record. Kept
/// in this module rather than `codeless-types` because the format is
/// for the proposals file specifically; the wire shape (the struct)
/// is what travels on the bus.
fn render_proposal_markdown(patch: &ScopePatch) -> String {
    let kind = match patch.kind {
        ScopePatchKind::Tighten => "tighten",
        ScopePatchKind::Loosen => "loosen",
    };
    let target = match patch.target {
        ScopePatchTarget::ClaudeMd => "claude-md",
        ScopePatchTarget::JobScopeMd => "job-scope-md",
        ScopePatchTarget::JobWorkflowMd => "job-workflow-md",
        ScopePatchTarget::JobClaudeMd => "job-claude-md",
    };
    let mut out = String::new();
    out.push_str("\n## ");
    out.push_str(&patch.id.to_string());
    out.push_str("\n\n");
    out.push_str(&format!(
        "- kind: {kind}\n- target: {target}\n- target-path: {}\n- review_id: {}\n- stage_id: {}\n- has_predicate: {}\n",
        patch.target_path, patch.review_id, patch.stage_id, patch.has_predicate,
    ));
    if let Some(ev) = patch.evidence_stage_id {
        out.push_str(&format!("- evidence_stage_id: {ev}\n"));
    }
    out.push_str("\n### Rationale\n\n");
    out.push_str(&patch.rationale);
    out.push_str("\n\n### Body\n\n");
    out.push_str(&patch.body);
    if !patch.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Parser intermediate — the four-field bag we hand to the publisher.
/// Held as a private struct (not the public `ScopePatch`) so the
/// `id` / `review_id` / `stage_id` fields are filled by the emit
/// path rather than the parser, which has no access to them.
#[derive(Debug)]
struct ParsedPatch {
    kind: ScopePatchKind,
    target: ScopePatchTarget,
    target_path: String,
    rationale: String,
    body: String,
    has_predicate: bool,
    evidence_stage_id: Option<StageId>,
}

enum ParseResult {
    None,
    One(ParsedPatch),
    Multiple,
    Malformed(String),
}

const BEGIN_MARKER: &str = "SCOPE-PATCH-BEGIN";
const END_MARKER: &str = "SCOPE-PATCH-END";

/// Walk the handover body line by line, isolating `BEGIN` / `END`
/// blocks. Returns `Multiple` as soon as the second `BEGIN` is seen
/// so the caller can short-circuit. `Malformed` is reported on a
/// missing `END`, a missing required key, or an unrecognised
/// discriminant value.
fn parse_blocks(body: &str) -> ParseResult {
    let mut current: Option<Vec<String>> = None;
    let mut blocks: Vec<Vec<String>> = Vec::new();
    for raw in body.lines() {
        let trimmed = raw.trim();
        if trimmed == BEGIN_MARKER {
            if current.is_some() {
                return ParseResult::Malformed(
                    "nested SCOPE-PATCH-BEGIN before SCOPE-PATCH-END".into(),
                );
            }
            current = Some(Vec::new());
            continue;
        }
        if trimmed == END_MARKER {
            match current.take() {
                Some(lines) => {
                    blocks.push(lines);
                    if blocks.len() > 1 {
                        return ParseResult::Multiple;
                    }
                }
                None => {
                    return ParseResult::Malformed(
                        "SCOPE-PATCH-END without matching SCOPE-PATCH-BEGIN".into(),
                    );
                }
            }
            continue;
        }
        if let Some(buf) = current.as_mut() {
            buf.push(raw.to_string());
        }
    }
    if current.is_some() {
        return ParseResult::Malformed("SCOPE-PATCH-BEGIN without closing END".into());
    }
    match blocks.len() {
        0 => ParseResult::None,
        1 => match parse_one(&blocks[0]) {
            Ok(p) => ParseResult::One(p),
            Err(reason) => ParseResult::Malformed(reason),
        },
        _ => ParseResult::Multiple,
    }
}

/// Parse one block's body. Each line is either `key: value` or, for
/// the `body:` key, a `body:` line followed by every subsequent line
/// captured verbatim (the body may be free-form prose or a diff
/// fragment). Lines outside a recognised key are ignored — the model
/// is free to emit commentary inside the block.
fn parse_one(lines: &[String]) -> Result<ParsedPatch, String> {
    let mut kind: Option<ScopePatchKind> = None;
    let mut target: Option<ScopePatchTarget> = None;
    let mut target_path: Option<String> = None;
    let mut rationale: Option<String> = None;
    let mut has_predicate = false;
    let mut evidence: Option<StageId> = None;
    let mut body_buf: Option<Vec<String>> = None;

    for line in lines {
        if let Some(buf) = body_buf.as_mut() {
            buf.push(line.clone());
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "kind" => {
                kind = Some(match value.to_ascii_lowercase().as_str() {
                    "tighten" => ScopePatchKind::Tighten,
                    "loosen" => ScopePatchKind::Loosen,
                    other => return Err(format!("unknown kind '{other}'")),
                });
            }
            "target" => {
                target = Some(match value.to_ascii_lowercase().as_str() {
                    "claude-md" => ScopePatchTarget::ClaudeMd,
                    "job-scope-md" => ScopePatchTarget::JobScopeMd,
                    "job-workflow-md" => ScopePatchTarget::JobWorkflowMd,
                    "job-claude-md" => ScopePatchTarget::JobClaudeMd,
                    other => return Err(format!("unknown target '{other}'")),
                });
            }
            "target-path" | "target_path" => {
                target_path = Some(value.to_string());
            }
            "rationale" => {
                rationale = Some(value.to_string());
            }
            "predicate" | "has-predicate" | "has_predicate" => {
                has_predicate = matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "1");
            }
            "evidence-stage-id" | "evidence_stage_id" | "evidence" => {
                evidence = Some(
                    StageId::from_str(value)
                        .map_err(|e| format!("evidence_stage_id is not a ULID: {e}"))?,
                );
            }
            "body" => {
                // Body captures everything after the `body:` line.
                // A same-line tail (`body: one-liner`) is honoured as
                // the first line so single-line bodies don't need a
                // continuation.
                let mut buf = Vec::new();
                if !value.is_empty() {
                    buf.push(value.to_string());
                }
                body_buf = Some(buf);
            }
            _ => {}
        }
    }

    let kind = kind.ok_or_else(|| "missing required key: kind".to_string())?;
    let target = target.ok_or_else(|| "missing required key: target".to_string())?;
    let target_path = target_path.ok_or_else(|| "missing required key: target-path".to_string())?;
    let rationale = rationale.ok_or_else(|| "missing required key: rationale".to_string())?;
    let body = body_buf
        .map(|b| b.join("\n"))
        .ok_or_else(|| "missing required key: body".to_string())?;

    Ok(ParsedPatch {
        kind,
        target,
        target_path,
        rationale,
        body,
        has_predicate,
        evidence_stage_id: evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{Event, EventEnvelope};
    use sqlx::SqlitePool;

    async fn make_bus() -> EventBus {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::MIGRATOR.run(&pool).await.unwrap();
        EventBus::new(pool, 32)
    }

    fn body_with(block: &str) -> String {
        format!("PASS: gate holds\n\n{block}\n")
    }

    fn well_formed_tighten() -> String {
        let mut s = String::new();
        s.push_str(BEGIN_MARKER);
        s.push('\n');
        s.push_str(
            "kind: tighten\n\
             target: claude-md\n\
             target-path: codeless/CLAUDE.md\n\
             rationale: R4 should explicitly auto-FAIL stages that edit files outside Done\n\
             predicate: true\n\
             body: append the sentence to R4 in codeless/CLAUDE.md\n",
        );
        s.push_str(END_MARKER);
        s
    }

    #[test]
    fn parses_well_formed_tighten() {
        let body = body_with(&well_formed_tighten());
        match parse_blocks(&body) {
            ParseResult::One(p) => {
                assert_eq!(p.kind, ScopePatchKind::Tighten);
                assert_eq!(p.target, ScopePatchTarget::ClaudeMd);
                assert_eq!(p.target_path, "codeless/CLAUDE.md");
                assert!(p.has_predicate);
                assert!(p.evidence_stage_id.is_none());
                assert!(p.rationale.contains("auto-FAIL"));
                assert!(p.body.contains("append the sentence"));
            }
            other => panic!(
                "expected One, got {other:?}",
                other = match other {
                    ParseResult::None => "None".to_string(),
                    ParseResult::Multiple => "Multiple".to_string(),
                    ParseResult::Malformed(r) => format!("Malformed({r})"),
                    ParseResult::One(_) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn no_block_is_no_block() {
        match parse_blocks("PASS: nothing to propose here\n") {
            ParseResult::None => {}
            _ => panic!("expected None"),
        }
    }

    #[test]
    fn two_blocks_short_circuit_to_multiple() {
        let body = format!("{}\n{}\n", well_formed_tighten(), well_formed_tighten());
        match parse_blocks(&body) {
            ParseResult::Multiple => {}
            _ => panic!("expected Multiple"),
        }
    }

    #[test]
    fn missing_required_key_is_malformed() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: loosen\n\
             rationale: drop the rule\n\
             body: -- delete --\n\
             {END_MARKER}"
        );
        match parse_blocks(&block) {
            ParseResult::Malformed(r) => assert!(r.contains("target")),
            _ => panic!("expected Malformed"),
        }
    }

    #[test]
    fn unknown_kind_is_malformed() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: rephrase\n\
             target: claude-md\n\
             target-path: x\n\
             rationale: y\n\
             body: z\n\
             {END_MARKER}"
        );
        match parse_blocks(&block) {
            ParseResult::Malformed(r) => assert!(r.contains("rephrase")),
            _ => panic!("expected Malformed"),
        }
    }

    #[test]
    fn begin_without_end_is_malformed() {
        let block = format!("{BEGIN_MARKER}\nkind: tighten\n");
        match parse_blocks(&block) {
            ParseResult::Malformed(r) => assert!(r.contains("without closing END")),
            _ => panic!("expected Malformed"),
        }
    }

    #[test]
    fn body_captures_multiline_remainder() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: loosen\n\
             target: job-scope-md\n\
             target-path: .codeless/jobs/x/SCOPE.md\n\
             rationale: prose-only loosening\n\
             evidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
             body:\n\
             delete paragraph one\n\
             delete paragraph two\n\
             {END_MARKER}"
        );
        match parse_blocks(&block) {
            ParseResult::One(p) => {
                assert_eq!(p.body, "delete paragraph one\ndelete paragraph two");
                assert!(p.evidence_stage_id.is_some());
            }
            _ => panic!("expected One"),
        }
    }

    #[tokio::test]
    async fn emit_from_handover_emits_event_and_writes_file() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let review_id = ReviewId::new();
        let body = body_with(&well_formed_tighten());

        let mut sub = bus
            .subscribe_since(crate::SubscribeFilter::All, None)
            .await
            .unwrap();

        let outcome =
            emit_from_handover(&bus, tmp.path(), job_id, stage_id, review_id, &body).await;
        let emitted_id = match outcome {
            EmitOutcome::Emitted(id) => id,
            other => panic!("expected Emitted, got {other:?}"),
        };

        // The proposals file exists, carries the patch id, and was
        // written with the bootstrapping header on first append.
        let path = tmp.path().join("DOCS/SCOPE-PROPOSED.md");
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(written.contains("# Proposed scope patches"));
        assert!(written.contains(&emitted_id.to_string()));
        assert!(written.contains("- kind: tighten"));
        assert!(written.contains("- target: claude-md"));
        assert!(written.contains("- has_predicate: true"));

        // The event variant fired on the bus.
        let env = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            use futures_util::StreamExt;
            sub.next().await
        })
        .await
        .expect("event arrived")
        .expect("stream live")
        .expect("envelope ok");
        match env.event {
            Event::ScopePatchProposed {
                patch_id,
                kind,
                target,
                ..
            } => {
                assert_eq!(patch_id, emitted_id);
                assert_eq!(kind, ScopePatchKind::Tighten);
                assert_eq!(target, ScopePatchTarget::ClaudeMd);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        // Suppress the unused-var warning on the EventEnvelope import.
        let _: fn() -> Option<EventEnvelope> = || None;
    }

    #[tokio::test]
    async fn second_append_does_not_reprint_header() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let body = body_with(&well_formed_tighten());

        let _ = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body,
        )
        .await;
        let _ = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body,
        )
        .await;

        let written = tokio::fs::read_to_string(tmp.path().join("DOCS/SCOPE-PROPOSED.md"))
            .await
            .unwrap();
        let header_hits = written.matches("# Proposed scope patches").count();
        assert_eq!(header_hits, 1, "header should appear exactly once");
    }

    #[tokio::test]
    async fn no_block_returns_no_block_with_no_side_effects() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            JobId::new(),
            StageId::new(),
            ReviewId::new(),
            "PASS: nothing to propose\n",
        )
        .await;
        match outcome {
            EmitOutcome::NoBlock => {}
            other => panic!("expected NoBlock, got {other:?}"),
        }
        let exists = tokio::fs::try_exists(tmp.path().join("DOCS/SCOPE-PROPOSED.md"))
            .await
            .unwrap();
        assert!(!exists, "no proposals file should be created");
    }
}
