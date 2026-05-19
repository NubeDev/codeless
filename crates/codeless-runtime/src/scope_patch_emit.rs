//! REVIEW-stage `ScopePatch` parse, validation, persistence, and event
//! emission. Step 4 of the SESSION-MUTABLE-SCOPE ramp introduced this
//! as shadow-mode emission; Step 5 layered the parse-time guards
//! ("tightening requires predicate, loosening requires positive
//!   fixture + cited evidence stage, one patch per REVIEW, mutable-set
//!   membership, evidence verified against the cited stage's diff").
//!
//! A REVIEW stage that wants to suggest a rulebook change appends a
//! `SCOPE-PATCH-BEGIN` … `SCOPE-PATCH-END` block to its handover body
//! alongside the standard `PASS:` sentinel. The runtime hands the same
//! body to this module after the gate parses `PASS`. When a single
//! well-formed-and-valid block is present the runtime allocates a
//! `ScopePatchId`, appends a human-readable record to
//! `DOCS/SCOPE-PROPOSED.md`, and publishes a `ScopePatchProposed`
//! envelope. When more than one block is present, when the block does
//! not parse, or when the validator rejects the proposal, the caller
//! converts the outcome into a REVIEW-gate FAIL — Step 5's promotion
//! from Step 4's warn-only behaviour.
//!
//! "Shadow mode" still describes the merge path: nothing lands
//! automatically. The proposal accumulates as a file artifact (the
//! Step 6 approval CLI will walk that file) and the event envelope
//! gives the kill-criterion query a data source per
//! `SESSION-MUTABLE-SCOPE-DECISIONS.md` Q7. What Step 5 changes is the
//! gate's behaviour on a *bad* proposal: a malformed block no longer
//! slides past the gate as a warn-level log.
//!
//! Why a custom mini-format and not (say) JSON: the handover is
//! authored by a model into a markdown document a human will read. A
//! plain key:value block sits naturally next to the `PASS:` sentinel,
//! and the parser stays small enough to live in one file.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use codeless_types::{
    Event, Handover, JobId, ReviewId, ScopePatch, ScopePatchId, ScopePatchKind, ScopePatchTarget,
    StageId,
};

use crate::diff_verify::{verify_handover, DiffVerifyOutcome};
use crate::event_bus::EventBus;
use crate::handover::handover_path;
use crate::rule_bearing_files::{classify, FileClass};
use crate::time::now_ms;

/// Outcome of trying to extract a patch proposal from a REVIEW
/// handover body. Three of the variants — `MultipleBlocks`,
/// `Malformed`, and `Rejected` — are Step 5 FAIL reasons; the REVIEW
/// gate converts them into a `ReviewVerdict::Fail`. The remaining
/// variants (`Emitted`, `NoBlock`, `SideEffectFailed`) leave the
/// gate's verdict alone.
#[derive(Debug)]
pub enum EmitOutcome {
    /// A well-formed-and-valid proposal was parsed, written to
    /// `DOCS/SCOPE-PROPOSED.md`, and published as an event.
    Emitted(ScopePatchId),
    /// No `SCOPE-PATCH-BEGIN` / `END` markers in the body. The most
    /// common case — most REVIEW stages will pass without proposing
    /// a rulebook change. Gate verdict is unchanged.
    NoBlock,
    /// More than one block. Step 5 promotes this to a REVIEW-gate
    /// FAIL (the contract is one proposal per REVIEW so the approval
    /// UX can show one card per stage).
    MultipleBlocks,
    /// The block existed but parsing failed (missing required key,
    /// unknown discriminant). Gate FAILs with the reason.
    Malformed(String),
    /// The block parsed but validation rejected it: shape-of-target
    /// mismatch, missing predicate on `Tighten`, missing fixture or
    /// unverified evidence on `Loosen`, mutable-set violation. Gate
    /// FAILs with the reason.
    Rejected(String),
    /// Parsing and validation succeeded but a downstream side-effect
    /// (writing the proposals file, publishing the event) failed.
    /// Carried through as warn-level only; an I/O wobble must not
    /// fail a REVIEW gate whose handover otherwise cleared.
    SideEffectFailed(String),
}

/// Parse, validate, persist, and publish in one shot. Called by
/// `template_runner` after a REVIEW gate returns `Pass`; safe to call
/// with a non-REVIEW handover body (will return `NoBlock` and do
/// nothing).
///
/// `changed_paths` is the worktree's enumerated diff against the base
/// ref. The runtime computes it once via
/// `codeless_adapters_host::changed_files` before this is called; we
/// take it as a parameter so the evidence-verification path stays
/// testable without spawning `git`.
pub async fn emit_from_handover(
    bus: &EventBus,
    worktree: &Path,
    job_id: JobId,
    stage_id: StageId,
    review_id: ReviewId,
    handover_body: &str,
    changed_paths: &[String],
) -> EmitOutcome {
    let parsed = match parse_blocks(handover_body) {
        ParseResult::None => return EmitOutcome::NoBlock,
        ParseResult::Multiple => return EmitOutcome::MultipleBlocks,
        ParseResult::Malformed(reason) => return EmitOutcome::Malformed(reason),
        ParseResult::One(p) => p,
    };

    if let Err(reason) = validate_shape(&parsed) {
        return EmitOutcome::Rejected(reason);
    }
    if parsed.kind == ScopePatchKind::Loosen {
        if let Err(reason) = verify_loosen_evidence(worktree, job_id, &parsed, changed_paths).await
        {
            return EmitOutcome::Rejected(reason);
        }
    }

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

    if let Err(err) = append_to_proposals_file(worktree, &patch, &parsed).await {
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

/// Step 5 patch-shape rules — synchronous because every check is
/// against in-memory state. Evidence verification is async and lives
/// in [`verify_loosen_evidence`].
fn validate_shape(p: &ParsedPatch) -> Result<(), String> {
    validate_target(&p.target, &p.target_path)?;
    match p.kind {
        ScopePatchKind::Tighten => validate_tighten(p)?,
        ScopePatchKind::Loosen => validate_loosen_shape(p)?,
    }
    Ok(())
}

/// Confirm the patch target names a path that is (a) in the mutable
/// set, (b) shape-consistent with the declared `target` discriminant.
/// The mutable set comes from `rule_bearing_files::classify`: a
/// rulebook-classified path is a mutable-via-REVIEW target;
/// wire-format and review-queue paths are explicitly rejected.
fn validate_target(target: &ScopePatchTarget, target_path: &str) -> Result<(), String> {
    let path = PathBuf::from(target_path);
    let normalised = normalise(&path);
    if normalised.is_empty() {
        return Err("target-path is empty".into());
    }
    match classify(&path) {
        FileClass::Rulebook => {}
        FileClass::WireFormat => {
            return Err(format!(
                "target-path `{target_path}` is a wire-format file; \
                 wire formats change via schema_version bumps, not REVIEW patches"
            ));
        }
        FileClass::ReviewQueue => {
            return Err(format!(
                "target-path `{target_path}` is the review queue itself; \
                 it is appended by the runtime and mutated by the approval CLI only"
            ));
        }
        FileClass::Open => {
            return Err(format!(
                "target-path `{target_path}` is not a rulebook file; \
                 REVIEW patches may only edit the mutable rulebook set"
            ));
        }
    }
    let basename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let in_jobs = normalised.starts_with(".codeless/jobs/");
    let job_segment_present = in_jobs
        && normalised
            .strip_prefix(".codeless/jobs/")
            .map(|rest| rest.contains('/'))
            .unwrap_or(false);
    let consistent = match target {
        ScopePatchTarget::ClaudeMd => basename == "CLAUDE.md" && !in_jobs,
        ScopePatchTarget::JobScopeMd => job_segment_present && basename == "SCOPE.md",
        ScopePatchTarget::JobWorkflowMd => job_segment_present && basename == "WORKFLOW.md",
        ScopePatchTarget::JobClaudeMd => job_segment_present && basename == "CLAUDE.md",
    };
    if !consistent {
        return Err(format!(
            "target-path `{target_path}` does not match target discriminant `{}`",
            match target {
                ScopePatchTarget::ClaudeMd => "claude-md",
                ScopePatchTarget::JobScopeMd => "job-scope-md",
                ScopePatchTarget::JobWorkflowMd => "job-workflow-md",
                ScopePatchTarget::JobClaudeMd => "job-claude-md",
            }
        ));
    }
    Ok(())
}

/// `Tighten` requires either a paired predicate file (`predicate:
/// true`) or a reference to an existing predicate the patch sharpens
/// (`predicate-ref: <probe-name>`). Without one of these, a tightening
/// patch is prose-only — exactly the failure mode the Layer-1 floor
/// was built to prevent. An evidence stage on a `Tighten` is
/// nonsensical (a tightening rule cannot cite a stage whose diff it
/// would have blocked because the rule did not exist yet) and is
/// rejected explicitly.
fn validate_tighten(p: &ParsedPatch) -> Result<(), String> {
    if p.evidence_stage_id.is_some() {
        return Err("tighten patches do not carry evidence_stage_id; \
             evidence stages are a loosening-patch concept"
            .into());
    }
    if !p.has_predicate && p.predicate_ref.is_none() {
        return Err(
            "tighten patches require either `predicate: true` (a new predicate \
             lands in the same commit) or `predicate-ref: <probe-name>` (the \
             patch sharpens an existing predicate)"
                .into(),
        );
    }
    if p.has_predicate && p.predicate_ref.is_some() {
        return Err(
            "tighten patches set either `predicate: true` or `predicate-ref`, \
             not both — a new predicate is the new-predicate path"
                .into(),
        );
    }
    if let Some(probe) = p.predicate_ref.as_deref() {
        if !codeless_predicates::probe_names().contains(&probe) {
            return Err(format!(
                "predicate-ref `{probe}` is not a known probe; known probes are: {}",
                codeless_predicates::probe_names().join(", ")
            ));
        }
    }
    Ok(())
}

/// `Loosen` requires `evidence-stage-id` + a positive `fixture-ref`
/// pointing at a test in `crates/codeless-predicates/tests/` or
/// inside a probe's `#[cfg(test)] mod tests`. A loosen with no
/// fixture cannot demonstrate that the previously-failing case is
/// actually permitted by the relaxed rule. A loosen that carries
/// `predicate: true` confuses two concepts: predicate *deletion* on a
/// loosen rides the approving human's commit (decisions Q5), not the
/// proposal.
fn validate_loosen_shape(p: &ParsedPatch) -> Result<(), String> {
    if p.has_predicate {
        return Err(
            "loosen patches do not set `predicate: true`; predicate deletion \
             rides the approving human's commit (decisions Q5)"
                .into(),
        );
    }
    if p.predicate_ref.is_some() {
        return Err(
            "loosen patches do not set `predicate-ref`; the cited evidence \
             stage is the proof, not a predicate cross-reference"
                .into(),
        );
    }
    if p.evidence_stage_id.is_none() {
        return Err(
            "loosen patches require `evidence-stage-id: <ulid>` naming the \
             stage whose diff is the positive fixture"
                .into(),
        );
    }
    let Some(fixture) = p.fixture_ref.as_deref() else {
        return Err(
            "loosen patches require `fixture-ref: <path>` pointing at a \
             positive fixture in predicate tests"
                .into(),
        );
    };
    let trimmed = fixture.trim();
    if trimmed.is_empty() {
        return Err("fixture-ref is empty".into());
    }
    if !(trimmed.starts_with("crates/codeless-predicates/")
        || trimmed.starts_with(".codeless/jobs/"))
    {
        return Err(format!(
            "fixture-ref `{trimmed}` must live under \
             `crates/codeless-predicates/` (predicate tests) or \
             `.codeless/jobs/` (prose-rule fixture); other paths are not \
             a positive fixture for this ramp"
        ));
    }
    Ok(())
}

/// Verify the cited evidence stage *actually has a diff* by
/// reconciling its handover's `Done` paths with the worktree's
/// enumerated changed-file set. Presence of the handover file alone
/// is not enough — a stage that wrote a handover but produced no diff
/// cannot serve as a positive fixture for a loosening claim.
async fn verify_loosen_evidence(
    worktree: &Path,
    job_id: JobId,
    parsed: &ParsedPatch,
    changed_paths: &[String],
) -> Result<(), String> {
    let evidence_stage_id = parsed
        .evidence_stage_id
        .ok_or_else(|| "internal: verify_loosen_evidence called without evidence".to_string())?;
    let path = handover_path(worktree, job_id, evidence_stage_id);
    let body = tokio::fs::read_to_string(&path).await.map_err(|err| {
        format!(
            "evidence stage handover not readable at `{}`: {err}",
            path.display()
        )
    })?;
    let handover = Handover::from_markdown(&body).map_err(|err| {
        format!(
            "evidence stage handover unparseable at `{}`: {err}",
            path.display()
        )
    })?;
    match verify_handover(&handover, changed_paths, worktree) {
        DiffVerifyOutcome::Pass { .. } => Ok(()),
        DiffVerifyOutcome::NothingToVerify => Err(format!(
            "evidence stage `{evidence_stage_id}` handover names no path-shaped tokens; \
             cannot verify against its diff"
        )),
        DiffVerifyOutcome::Fail { missing } => {
            let names: Vec<String> = missing.iter().map(|m| m.claimed.clone()).collect();
            Err(format!(
                "evidence stage `{evidence_stage_id}` cites paths absent from the worktree diff: {}",
                names.join(", ")
            ))
        }
    }
}

/// Append a human-readable record of the proposal to
/// `DOCS/SCOPE-PROPOSED.md`. The file is created if absent. The
/// format is deliberately append-only and lightly structured: a
/// `## <patch_id>` heading plus bulleted metadata (including the
/// parser-only `predicate-ref` / `fixture-ref` cross-references the
/// approval UX needs) followed by Rationale / Body sections.
async fn append_to_proposals_file(
    worktree: &Path,
    patch: &ScopePatch,
    parsed: &ParsedPatch,
) -> std::io::Result<()> {
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
    record.push_str(&render_proposal_markdown(patch, parsed, now_ms().as_i64()));

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

/// Render one proposal as the human-readable markdown record.
fn render_proposal_markdown(
    patch: &ScopePatch,
    parsed: &ParsedPatch,
    proposed_at_ms: i64,
) -> String {
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
    out.push_str(&format!("- proposed_at: {proposed_at_ms}\n"));
    if let Some(pr) = &parsed.predicate_ref {
        out.push_str(&format!("- predicate-ref: {pr}\n"));
    }
    if let Some(fx) = &parsed.fixture_ref {
        out.push_str(&format!("- fixture-ref: {fx}\n"));
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

/// Parser intermediate — the bag we hand to the validator and emitter.
/// Two of its fields (`predicate_ref`, `fixture_ref`) are
/// parser-internal: they are validation cross-references, not wire
/// fields, so they live here rather than on `ScopePatch`.
#[derive(Debug)]
struct ParsedPatch {
    kind: ScopePatchKind,
    target: ScopePatchTarget,
    target_path: String,
    rationale: String,
    body: String,
    has_predicate: bool,
    evidence_stage_id: Option<StageId>,
    predicate_ref: Option<String>,
    fixture_ref: Option<String>,
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
/// captured verbatim.
fn parse_one(lines: &[String]) -> Result<ParsedPatch, String> {
    let mut kind: Option<ScopePatchKind> = None;
    let mut target: Option<ScopePatchTarget> = None;
    let mut target_path: Option<String> = None;
    let mut rationale: Option<String> = None;
    let mut has_predicate = false;
    let mut evidence: Option<StageId> = None;
    let mut predicate_ref: Option<String> = None;
    let mut fixture_ref: Option<String> = None;
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
            "predicate-ref" | "predicate_ref" => {
                predicate_ref = Some(value.to_string());
            }
            "fixture-ref" | "fixture_ref" => {
                fixture_ref = Some(value.to_string());
            }
            "evidence-stage-id" | "evidence_stage_id" | "evidence" => {
                evidence = Some(
                    StageId::from_str(value)
                        .map_err(|e| format!("evidence_stage_id is not a ULID: {e}"))?,
                );
            }
            "body" => {
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
        predicate_ref,
        fixture_ref,
    })
}

/// Normalise a repo-relative path for prefix/string checks. Mirrors
/// `rule_bearing_files::normalise` for the same reasons (Windows
/// slashes, leading `./`).
fn normalise(path: &Path) -> String {
    let mut out = String::new();
    for (i, comp) in path.components().enumerate() {
        let part = match comp {
            std::path::Component::Normal(s) => s.to_string_lossy().to_string(),
            std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => "..".to_string(),
            std::path::Component::RootDir => continue,
            std::path::Component::Prefix(p) => p.as_os_str().to_string_lossy().to_string(),
        };
        if i > 0 && !out.is_empty() {
            out.push('/');
        }
        out.push_str(&part);
    }
    out.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{Event, EventEnvelope, Handover};
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
                "expected One, got {}",
                match other {
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

    fn parsed(extra: &str) -> ParsedPatch {
        // `body:` consumes every remaining line in the block, so extras
        // must precede it. Tests use this helper to add metadata
        // (predicate / predicate-ref / evidence) that the validator
        // reads before the body capture begins.
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             target: claude-md\n\
             target-path: codeless/CLAUDE.md\n\
             rationale: r\n\
             {extra}\
             body: b\n\
             {END_MARKER}"
        );
        match parse_blocks(&block) {
            ParseResult::One(p) => p,
            other => panic!(
                "expected One, got {}",
                match other {
                    ParseResult::None => "None".into(),
                    ParseResult::Multiple => "Multiple".into(),
                    ParseResult::Malformed(r) => format!("Malformed({r})"),
                    ParseResult::One(_) => unreachable!(),
                }
            ),
        }
    }

    #[test]
    fn tighten_without_predicate_or_predicate_ref_rejected() {
        let p = parsed("");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("predicate"), "got: {err}");
    }

    #[test]
    fn tighten_with_predicate_ref_to_known_probe_passes() {
        let p = parsed("predicate-ref: no-process-spawn-outside-adapters-host\n");
        validate_shape(&p).expect("known probe");
    }

    #[test]
    fn tighten_with_unknown_predicate_ref_rejected() {
        let p = parsed("predicate-ref: no-such-probe\n");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("no-such-probe"));
    }

    #[test]
    fn tighten_with_both_predicate_and_predicate_ref_rejected() {
        let p = parsed("predicate: true\npredicate-ref: no-emojis-in-source\n");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("not both"));
    }

    #[test]
    fn tighten_with_evidence_rejected() {
        let p = parsed("predicate: true\nevidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("evidence"));
    }

    fn loosen_block(extra: &str) -> ParsedPatch {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: loosen\n\
             target: job-scope-md\n\
             target-path: .codeless/jobs/x/SCOPE.md\n\
             rationale: r\n\
             {extra}\
             body: b\n\
             {END_MARKER}"
        );
        match parse_blocks(&block) {
            ParseResult::One(p) => p,
            _ => panic!("expected One"),
        }
    }

    #[test]
    fn loosen_without_evidence_rejected() {
        let p = loosen_block("fixture-ref: crates/codeless-predicates/tests/x.rs\n");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("evidence-stage-id"));
    }

    #[test]
    fn loosen_without_fixture_rejected() {
        let p = loosen_block("evidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n");
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("fixture-ref"));
    }

    #[test]
    fn loosen_with_predicate_rejected() {
        let p = loosen_block(
            "evidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
             fixture-ref: crates/codeless-predicates/tests/x.rs\n\
             predicate: true\n",
        );
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("predicate deletion") || err.contains("predicate: true"));
    }

    #[test]
    fn loosen_with_fixture_outside_predicates_or_jobs_rejected() {
        let p = loosen_block(
            "evidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
             fixture-ref: crates/codeless-runtime/tests/something.rs\n",
        );
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("fixture-ref"));
    }

    #[test]
    fn loosen_well_formed_passes_shape() {
        let p = loosen_block(
            "evidence: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
             fixture-ref: crates/codeless-predicates/tests/positive_fixture.rs\n",
        );
        validate_shape(&p).expect("loosen shape valid");
    }

    #[test]
    fn target_path_wire_format_rejected() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             target: claude-md\n\
             target-path: DOCS/JOB-MODEL.md\n\
             rationale: r\n\
             predicate: true\n\
             body: b\n\
             {END_MARKER}"
        );
        let ParseResult::One(p) = parse_blocks(&block) else {
            panic!("parse expected One")
        };
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("wire-format"));
    }

    #[test]
    fn target_path_unrelated_file_rejected() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             target: claude-md\n\
             target-path: src/main.rs\n\
             rationale: r\n\
             predicate: true\n\
             body: b\n\
             {END_MARKER}"
        );
        let ParseResult::One(p) = parse_blocks(&block) else {
            panic!("parse expected One")
        };
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("not a rulebook"));
    }

    #[test]
    fn target_path_mismatched_kind_rejected() {
        // Target says `claude-md` but path is a per-job SCOPE.md.
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             target: claude-md\n\
             target-path: .codeless/jobs/foo/SCOPE.md\n\
             rationale: r\n\
             predicate: true\n\
             body: b\n\
             {END_MARKER}"
        );
        let ParseResult::One(p) = parse_blocks(&block) else {
            panic!("parse expected One")
        };
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("does not match target discriminant"));
    }

    #[test]
    fn target_path_review_queue_rejected() {
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             target: claude-md\n\
             target-path: DOCS/SCOPE-PROPOSED.md\n\
             rationale: r\n\
             predicate: true\n\
             body: b\n\
             {END_MARKER}"
        );
        let ParseResult::One(p) = parse_blocks(&block) else {
            panic!("parse expected One")
        };
        let err = validate_shape(&p).unwrap_err();
        assert!(err.contains("review queue"));
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
            emit_from_handover(&bus, tmp.path(), job_id, stage_id, review_id, &body, &[]).await;
        let emitted_id = match outcome {
            EmitOutcome::Emitted(id) => id,
            other => panic!("expected Emitted, got {other:?}"),
        };

        let path = tmp.path().join("DOCS/SCOPE-PROPOSED.md");
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(written.contains("# Proposed scope patches"));
        assert!(written.contains(&emitted_id.to_string()));
        assert!(written.contains("- kind: tighten"));
        assert!(written.contains("- target: claude-md"));
        assert!(written.contains("- has_predicate: true"));

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
            &[],
        )
        .await;
        let _ = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body,
            &[],
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
            &[],
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

    #[tokio::test]
    async fn loosen_emits_only_when_evidence_diff_matches() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let evidence_stage = StageId::new();

        // Stage an evidence handover so verify_loosen_evidence has
        // something to read.
        let h = Handover {
            done: vec!["touched `crates/codeless-predicates/src/probes/x.rs`".into()],
            next: vec!["next".into()],
            ..Default::default()
        };
        crate::handover::write_handover(tmp.path(), job_id, evidence_stage, &h)
            .await
            .unwrap();

        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: loosen\n\
             target: job-scope-md\n\
             target-path: .codeless/jobs/foo/SCOPE.md\n\
             rationale: drop the rule\n\
             evidence: {evidence_stage}\n\
             fixture-ref: crates/codeless-predicates/tests/fix.rs\n\
             body: delete paragraph\n\
             {END_MARKER}"
        );
        let body = body_with(&block);

        // First: the cited path is NOT in the diff → Rejected.
        // The diff DOES touch something under `crates/` so the
        // tokenizer admits the evidence-bullet token via the diff-
        // prefix branch (otherwise the new shape filter would drop
        // the cited path before the diff-presence check ever runs and
        // the rejection reason would be "names no path-shaped tokens"
        // instead of the "absent from the worktree diff" outcome the
        // rejection-path operator message depends on).
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body,
            &["crates/codeless-predicates/src/probes/other.rs".to_string()],
        )
        .await;
        match outcome {
            EmitOutcome::Rejected(r) => {
                assert!(r.contains("absent from the worktree diff"), "got: {r}");
            }
            other => panic!("expected Rejected, got {other:?}"),
        }

        // Then: same block, this time the diff contains the cited path
        // → Emitted.
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body,
            &["crates/codeless-predicates/src/probes/x.rs".to_string()],
        )
        .await;
        assert!(
            matches!(outcome, EmitOutcome::Emitted(_)),
            "expected Emitted, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn loosen_with_missing_evidence_handover_rejected() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let evidence_stage = StageId::new();
        // No handover written — the cited stage does not exist.

        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: loosen\n\
             target: job-scope-md\n\
             target-path: .codeless/jobs/foo/SCOPE.md\n\
             rationale: r\n\
             evidence: {evidence_stage}\n\
             fixture-ref: crates/codeless-predicates/tests/x.rs\n\
             body: b\n\
             {END_MARKER}"
        );
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            job_id,
            StageId::new(),
            ReviewId::new(),
            &body_with(&block),
            &[],
        )
        .await;
        match outcome {
            EmitOutcome::Rejected(r) => {
                assert!(r.contains("not readable"), "got: {r}");
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn multiple_blocks_returns_multiple_blocks() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let two = format!("{}\n{}\n", well_formed_tighten(), well_formed_tighten());
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            JobId::new(),
            StageId::new(),
            ReviewId::new(),
            &two,
            &[],
        )
        .await;
        assert!(matches!(outcome, EmitOutcome::MultipleBlocks));
    }

    #[tokio::test]
    async fn missing_required_key_returns_malformed() {
        let bus = make_bus().await;
        let tmp = tempfile::tempdir().unwrap();
        let block = format!(
            "{BEGIN_MARKER}\n\
             kind: tighten\n\
             rationale: r\n\
             body: b\n\
             {END_MARKER}"
        );
        let outcome = emit_from_handover(
            &bus,
            tmp.path(),
            JobId::new(),
            StageId::new(),
            ReviewId::new(),
            &block,
            &[],
        )
        .await;
        assert!(matches!(outcome, EmitOutcome::Malformed(_)));
    }
}
