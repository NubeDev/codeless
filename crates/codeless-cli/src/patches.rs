//! `codeless patches {list,show,approve,reject,edit}` — Step 6 of the
//! SESSION-MUTABLE-SCOPE ramp. The CLI walks the proposed-patch queue
//! in `DOCS/SCOPE-PROPOSED.md`, surfaces metadata to the operator, and
//! produces human-authored git commits when a patch is approved or
//! rejected.
//!
//! Why this lives in the CLI and not the RPC surface: the proposals
//! file is a working-tree artifact (R4 — "patch approval lands as a
//! normal git commit, no new persistence store"), the human's editor
//! ergonomics matter, and the action is fundamentally local. A future
//! UI affordance would speak the same queue file through different
//! ergonomics; deferring that is the task's explicit "UI affordance
//! is deferred to a follow-up job".
//!
//! Process spawn lives in `codeless-adapters-host` per R1. Commits go
//! through [`codeless_adapters_host::commit_paths`]; the CLI never
//! shells out to `git` directly.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use codeless_adapters_host::{commit_paths, invoke_editor, pick_editor};
use codeless_runtime::scope_patch_queue::{
    load_queue, parse_queue, write_queue, Proposal, QueueError,
};
use codeless_types::{ScopePatchId, ScopePatchKind, ScopePatchTarget};

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// One line per proposed patch: `<id> <kind> <target> <target-path>`.
    /// Exit code is 0 even when the queue is empty so the verb composes
    /// with shell pipelines.
    List(RepoArgs),
    /// Print the full markdown record for one proposed patch.
    Show(IdArgs),
    /// Remove the patch entry from `DOCS/SCOPE-PROPOSED.md` and create
    /// a git commit that includes the queue edit, the target rulebook
    /// file the human edited, and any `--include`d predicate/fixture
    /// files. The commit body cites the evidence stage (Loosen) or the
    /// emitting stage (Tighten); the human is the commit author. The
    /// human must have made the rulebook edits on disk before running
    /// this command; we do not interpret the patch body.
    Approve(ApproveArgs),
    /// Remove the patch entry from `DOCS/SCOPE-PROPOSED.md` and create
    /// a git commit that records the rejection. No rulebook file is
    /// touched (it was never edited) so only `DOCS/SCOPE-PROPOSED.md`
    /// is staged.
    Reject(RejectArgs),
    /// Open the proposed patch in `$EDITOR` (or `$VISUAL`); on save,
    /// re-parse the edited block and replace the queue entry. No
    /// commit is produced — the operator typically follows up with
    /// `codeless patches approve`. Used to fix typos in a model-
    /// authored rationale or tweak the body's instructions before
    /// approving.
    Edit(IdArgs),
}

#[derive(Debug, Args)]
pub struct RepoArgs {
    /// Worktree root. Defaults to the current working directory. The
    /// proposals file is read from `<repo>/DOCS/SCOPE-PROPOSED.md`.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    #[command(flatten)]
    pub repo: RepoArgs,
    /// The patch's ULID, as printed by `codeless patches list`.
    pub id: String,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    #[command(flatten)]
    pub repo: RepoArgs,
    pub id: String,
    /// Repo-relative path to include in the approval commit alongside
    /// the queue edit and the patch's `target-path`. Repeat the flag
    /// to commit multiple paths (e.g. a new predicate file and its
    /// test fixture). Files outside the worktree root are rejected.
    #[arg(long = "include", value_name = "PATH")]
    pub include: Vec<PathBuf>,
    /// Override the auto-generated commit subject. The default is
    /// `scope-patch <kind>: <rationale>`; supply your own when the
    /// rationale is awkward as a commit subject.
    #[arg(long)]
    pub message: Option<String>,
}

#[derive(Debug, Args)]
pub struct RejectArgs {
    #[command(flatten)]
    pub repo: RepoArgs,
    pub id: String,
    /// Optional reason recorded in the rejection commit body. Audit
    /// trail only — the runtime does not act on it.
    #[arg(long)]
    pub reason: Option<String>,
}

pub fn handle(verb: Verb) -> Result<ExitCode> {
    match verb {
        Verb::List(args) => list(args),
        Verb::Show(args) => show(args),
        Verb::Approve(args) => approve(args),
        Verb::Reject(args) => reject(args),
        Verb::Edit(args) => edit(args),
    }
}

fn list(args: RepoArgs) -> Result<ExitCode> {
    match load_queue(&args.repo) {
        Ok(queue) => {
            for p in &queue.proposals {
                println!(
                    "{}\t{}\t{}\t{}",
                    p.id,
                    kind_str(p.kind),
                    target_str(p.target),
                    p.target_path
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(QueueError::Missing { .. }) => {
            eprintln!("no proposed patches: DOCS/SCOPE-PROPOSED.md does not exist yet");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => Err(e.into()),
    }
}

fn show(args: IdArgs) -> Result<ExitCode> {
    let id = parse_id(&args.id)?;
    let queue = load_queue(&args.repo.repo)?;
    let p = queue
        .find(id)
        .ok_or_else(|| anyhow!("no proposed patch with id `{id}`"))?;
    print!("{}", p.render());
    Ok(ExitCode::SUCCESS)
}

fn approve(args: ApproveArgs) -> Result<ExitCode> {
    let id = parse_id(&args.id)?;
    let repo = args.repo.repo.clone();
    let queue = load_queue(&repo)?;
    let proposal = queue
        .find(id)
        .ok_or_else(|| anyhow!("no proposed patch with id `{id}`"))?
        .clone();
    let target_path = repo.join(&proposal.target_path);
    if !target_path.exists() {
        bail!(
            "target rulebook file `{}` does not exist; the human must edit it before approval",
            proposal.target_path
        );
    }
    let mut to_commit: Vec<PathBuf> = Vec::new();
    to_commit.push(repo.join("DOCS").join("SCOPE-PROPOSED.md"));
    to_commit.push(target_path);
    for extra in &args.include {
        let abs = if extra.is_absolute() {
            extra.clone()
        } else {
            repo.join(extra)
        };
        if !abs.exists() {
            bail!("--include path `{}` does not exist", abs.display());
        }
        if !abs.starts_with(&repo) {
            bail!(
                "--include path `{}` is outside the worktree root `{}`",
                abs.display(),
                repo.display()
            );
        }
        to_commit.push(abs);
    }

    // Remove the entry and rewrite the queue file before staging, so
    // `git add` sees the post-removal contents.
    let mut queue = queue;
    queue.remove(id).expect("present above");
    write_queue(&repo, &queue)?;

    let subject = args
        .message
        .clone()
        .unwrap_or_else(|| default_subject(&proposal));
    let body_lines = approve_commit_body(&proposal, &args.include);
    let full_message = combine_message(&subject, &body_lines);

    let made =
        commit_paths(&repo, &full_message, &to_commit).with_context(|| "git commit for approve")?;
    if !made {
        eprintln!(
            "warning: git reported nothing to commit. The queue entry was removed but the \
             target file may already match HEAD — run `git status` to verify."
        );
    }
    println!("approved {id}");
    Ok(ExitCode::SUCCESS)
}

fn reject(args: RejectArgs) -> Result<ExitCode> {
    let id = parse_id(&args.id)?;
    let repo = args.repo.repo.clone();
    let mut queue = load_queue(&repo)?;
    let proposal = queue
        .remove(id)
        .ok_or_else(|| anyhow!("no proposed patch with id `{id}`"))?;
    write_queue(&repo, &queue)?;

    let subject = format!("scope-patch reject: {}", short_rationale(&proposal));
    let body_lines = reject_commit_body(&proposal, args.reason.as_deref());
    let full = combine_message(&subject, &body_lines);
    let made = commit_paths(&repo, &full, &[repo.join("DOCS").join("SCOPE-PROPOSED.md")])
        .with_context(|| "git commit for reject")?;
    if !made {
        eprintln!(
            "warning: git reported nothing to commit — DOCS/SCOPE-PROPOSED.md may have been clean."
        );
    }
    println!("rejected {id}");
    Ok(ExitCode::SUCCESS)
}

fn edit(args: IdArgs) -> Result<ExitCode> {
    let id = parse_id(&args.id)?;
    let repo = args.repo.repo.clone();
    let queue = load_queue(&repo)?;
    let proposal = queue
        .find(id)
        .ok_or_else(|| anyhow!("no proposed patch with id `{id}`"))?
        .clone();

    let dir = tempfile::tempdir().context("create temp dir for editor")?;
    let tmp = dir.path().join(format!("{}.md", id));
    std::fs::write(&tmp, proposal.render()).context("seed temp file for editor")?;

    let editor_cmd = pick_editor()
        .ok_or_else(|| anyhow!("neither $VISUAL nor $EDITOR is set; cannot launch an editor"))?;
    let status = invoke_editor(&editor_cmd, &tmp)?;
    if !status.success() {
        bail!("editor `{editor_cmd}` exited with status {status}");
    }
    let edited = std::fs::read_to_string(&tmp).context("re-read edited file")?;
    let parsed = parse_queue(&format!("# Proposed scope patches\n\n{edited}"))
        .map_err(|e| anyhow!("edited buffer is not a parseable proposal block: {e}"))?;
    let new_proposal = parsed
        .proposals
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("edited buffer contains no proposal block"))?;
    if new_proposal.id != id {
        bail!(
            "edited buffer changed the proposal id from `{id}` to `{}`; abort",
            new_proposal.id
        );
    }

    let mut queue = queue;
    if !queue.replace(new_proposal) {
        bail!("proposal `{id}` vanished from queue during edit");
    }
    write_queue(&repo, &queue)?;
    println!("edited {id}");
    Ok(ExitCode::SUCCESS)
}

fn parse_id(s: &str) -> Result<ScopePatchId> {
    ScopePatchId::from_str(s).map_err(|e| anyhow!("invalid patch id `{s}`: {e}"))
}

fn kind_str(k: ScopePatchKind) -> &'static str {
    match k {
        ScopePatchKind::Tighten => "tighten",
        ScopePatchKind::Loosen => "loosen",
    }
}

fn target_str(t: ScopePatchTarget) -> &'static str {
    match t {
        ScopePatchTarget::ClaudeMd => "claude-md",
        ScopePatchTarget::JobScopeMd => "job-scope-md",
        ScopePatchTarget::JobWorkflowMd => "job-workflow-md",
        ScopePatchTarget::JobClaudeMd => "job-claude-md",
    }
}

/// Short single-line rationale safe to use as a commit subject. The
/// stored rationale may be a full sentence with trailing punctuation;
/// commit subjects conventionally do not end in `.` and stay under
/// 72 characters.
fn short_rationale(p: &Proposal) -> String {
    let first_line = p.rationale.lines().next().unwrap_or("").trim();
    let trimmed = first_line.trim_end_matches('.');
    if trimmed.chars().count() <= 60 {
        trimmed.to_string()
    } else {
        let mut acc = String::new();
        for (i, c) in trimmed.chars().enumerate() {
            if i == 57 {
                acc.push_str("...");
                break;
            }
            acc.push(c);
        }
        acc
    }
}

fn default_subject(p: &Proposal) -> String {
    format!("scope-patch {}: {}", kind_str(p.kind), short_rationale(p))
}

fn approve_commit_body(p: &Proposal, includes: &[PathBuf]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Approved scope patch {}.", p.id));
    lines.push(String::new());
    lines.push(format!("- kind: {}", kind_str(p.kind)));
    lines.push(format!("- target: {}", target_str(p.target)));
    lines.push(format!("- target-path: {}", p.target_path));
    if let Some(ev) = p.evidence_stage_id {
        lines.push(format!("- evidence_stage_id: {ev}"));
    }
    if let Some(pr) = &p.predicate_ref {
        lines.push(format!("- predicate-ref: {pr}"));
    }
    if let Some(fx) = &p.fixture_ref {
        lines.push(format!("- fixture-ref: {fx}"));
    }
    if !includes.is_empty() {
        lines.push("- additional files committed:".into());
        for inc in includes {
            lines.push(format!("    - {}", inc.display()));
        }
    }
    lines.push(String::new());
    lines.push("Rationale:".into());
    for r in p.rationale.lines() {
        lines.push(format!("  {r}"));
    }
    lines
}

fn reject_commit_body(p: &Proposal, reason: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Rejected scope patch {}.", p.id));
    lines.push(String::new());
    lines.push(format!("- kind: {}", kind_str(p.kind)));
    lines.push(format!("- target: {}", target_str(p.target)));
    lines.push(format!("- target-path: {}", p.target_path));
    if let Some(ev) = p.evidence_stage_id {
        lines.push(format!("- evidence_stage_id: {ev}"));
    }
    if let Some(reason) = reason {
        lines.push(String::new());
        lines.push("Reason:".into());
        for r in reason.lines() {
            lines.push(format!("  {r}"));
        }
    }
    lines
}

fn combine_message(subject: &str, body_lines: &[String]) -> String {
    let mut out = String::from(subject);
    if !body_lines.is_empty() {
        out.push_str("\n\n");
        out.push_str(&body_lines.join("\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{ReviewId, ScopePatch, StageId};

    fn sample_proposal(kind: ScopePatchKind, target_path: &str) -> Proposal {
        let p = ScopePatch {
            id: ScopePatchId::new(),
            review_id: ReviewId::new(),
            stage_id: StageId::new(),
            kind,
            target: ScopePatchTarget::ClaudeMd,
            target_path: target_path.into(),
            rationale: "R4 should explicitly auto-FAIL stages that edit files outside Done".into(),
            body: "append the sentence to R4".into(),
            has_predicate: kind == ScopePatchKind::Tighten,
            evidence_stage_id: if kind == ScopePatchKind::Loosen {
                Some(StageId::new())
            } else {
                None
            },
        };
        Proposal {
            id: p.id,
            kind: p.kind,
            target: p.target,
            target_path: p.target_path,
            rationale: p.rationale,
            body: p.body,
            has_predicate: p.has_predicate,
            evidence_stage_id: p.evidence_stage_id,
            predicate_ref: Some("no-emojis-in-source".into()),
            fixture_ref: None,
            span: (0, 0),
        }
    }

    #[test]
    fn short_rationale_strips_period_and_caps_length() {
        let mut p = sample_proposal(ScopePatchKind::Tighten, "codeless/CLAUDE.md");
        p.rationale = "short reason.".into();
        assert_eq!(short_rationale(&p), "short reason");

        p.rationale = "x".repeat(120);
        let out = short_rationale(&p);
        assert!(out.ends_with("..."));
        assert!(out.chars().count() <= 60);
    }

    #[test]
    fn default_subject_includes_kind_and_rationale() {
        let p = sample_proposal(ScopePatchKind::Tighten, "codeless/CLAUDE.md");
        let s = default_subject(&p);
        assert!(s.starts_with("scope-patch tighten:"));
        assert!(s.contains("R4"));
    }

    #[test]
    fn approve_commit_body_cites_evidence_when_loosen() {
        let p = sample_proposal(ScopePatchKind::Loosen, "codeless/CLAUDE.md");
        let lines = approve_commit_body(&p, &[]);
        let joined = lines.join("\n");
        assert!(joined.contains("Approved scope patch"));
        assert!(joined.contains("evidence_stage_id:"));
        assert!(joined.contains("Rationale:"));
    }

    #[test]
    fn approve_commit_body_lists_includes() {
        let p = sample_proposal(ScopePatchKind::Tighten, "codeless/CLAUDE.md");
        let extras = vec![PathBuf::from("crates/codeless-predicates/src/probes/x.rs")];
        let body = approve_commit_body(&p, &extras).join("\n");
        assert!(body.contains("additional files committed"));
        assert!(body.contains("crates/codeless-predicates/src/probes/x.rs"));
    }

    #[test]
    fn reject_commit_body_records_reason() {
        let p = sample_proposal(ScopePatchKind::Tighten, "codeless/CLAUDE.md");
        let body = reject_commit_body(&p, Some("overconstrains")).join("\n");
        assert!(body.contains("Rejected scope patch"));
        assert!(body.contains("Reason:"));
        assert!(body.contains("overconstrains"));
    }
}
