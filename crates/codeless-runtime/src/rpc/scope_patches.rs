//! UI-driven approve / reject / edit of proposed scope patches. Wraps
//! the same in-runtime queue helpers (`scope_patch_queue`) the
//! `codeless patches` CLI uses, but stages commit creation behind
//! `codeless-adapters-host::commit_paths` (R1: process spawn lives in
//! the host crate only) and resolves author identity from the repo-
//! local `git config user.{name,email}` rather than asking the UI to
//! send one.
//!
//! Idempotency is load-bearing: two windows can show the same proposal
//! and the second-to-click must not see an error. On a second call the
//! queue file no longer carries the patch; we scan `git log` for the
//! resolution commit and return `ScopePatchActionResult::AlreadyResolved`
//! so the UI flips its row to the resolved view without a red toast.
//!
//! Approval and rejection emit `ScopePatchApproved` /
//! `ScopePatchRejected` envelopes so cross-window subscribers can
//! invalidate caches (Dependency #3 in `DOCS/SCOPE-MUTABLE-UI.md`).
//! Edit is silent — it does not change the resolution state.

use std::path::{Path, PathBuf};

use codeless_adapters_host::{
    commit_paths, find_patch_resolution, git_revert, head_sha, PriorPatchResolution,
};
use codeless_rpc::{
    ApproveScopePatchArgs, EditScopePatchArgs, RejectScopePatchArgs, RevertScopePatchArgs,
    RevertScopePatchResult, RpcError, RpcResult, ScopePatchActionResult, ScopePatchResolution,
};
use codeless_types::{Event, Repo, RepoId, ScopePatchId, ScopePatchKind, ScopePatchTarget};

use super::InProcessRpc;
use crate::scope_patch_queue::{load_queue, parse_queue, write_queue, Proposal, QueueError};
use crate::time::now_ms;

/// Marker appended to every UI-driven approval / rejection commit body
/// so `git log` distinguishes UI clicks from `codeless patches` CLI
/// invocations. Lives in this module rather than on the CLI side so the
/// runtime is the single source of truth for the wire shape.
const UI_TRAILER: &str = "Codeless-Approved-By: ui";

pub(super) async fn approve_scope_patch(
    rpc: &InProcessRpc,
    args: ApproveScopePatchArgs,
) -> RpcResult<ScopePatchActionResult> {
    let repo = resolve_repo(rpc, args.repo_id).await?;
    let repo_path = PathBuf::from(&repo.local_path);
    let queue = match load_queue(&repo_path) {
        Ok(q) => q,
        Err(QueueError::Missing { .. }) => {
            return resolved_or_not_found(&repo_path, args.patch_id);
        }
        Err(e) => return Err(queue_err(e)),
    };

    let proposal = match queue.find(args.patch_id) {
        Some(p) => p.clone(),
        None => return resolved_or_not_found(&repo_path, args.patch_id),
    };

    let target_abs = repo_path.join(&proposal.target_path);
    if !target_abs.exists() {
        return Err(RpcError::InvalidArgument(format!(
            "target rulebook file `{}` does not exist; \
             edit it on disk before approving the patch",
            proposal.target_path
        )));
    }

    let mut to_commit: Vec<PathBuf> = Vec::new();
    to_commit.push(repo_path.join("DOCS").join("SCOPE-PROPOSED.md"));
    to_commit.push(target_abs);
    for extra in &args.include {
        let extra_path = PathBuf::from(extra);
        let abs = if extra_path.is_absolute() {
            extra_path.clone()
        } else {
            repo_path.join(&extra_path)
        };
        if !abs.exists() {
            return Err(RpcError::InvalidArgument(format!(
                "include path `{}` does not exist",
                abs.display()
            )));
        }
        if !abs.starts_with(&repo_path) {
            return Err(RpcError::InvalidArgument(format!(
                "include path `{}` is outside the worktree root `{}`",
                abs.display(),
                repo_path.display()
            )));
        }
        to_commit.push(abs);
    }

    let mut queue = queue;
    queue.remove(args.patch_id).expect("present above");
    write_queue(&repo_path, &queue).map_err(queue_err)?;

    let subject = args
        .message
        .clone()
        .unwrap_or_else(|| default_subject(&proposal));
    let body = build_approve_body(&proposal, &args.include);
    let full = combine_message(&subject, &body);

    commit_paths(&repo_path, &full, &to_commit)
        .map_err(|e| RpcError::Internal(format!("git commit: {e}")))?;

    let commit_sha =
        head_sha(&repo_path).map_err(|e| RpcError::Internal(format!("git rev-parse HEAD: {e}")))?;

    let stage_id = synthetic_stage_id();
    let review_id = synthetic_review_id();
    rpc.bus
        .publish(
            None,
            Some(stage_id),
            None,
            Event::ScopePatchApproved {
                stage_id,
                review_id,
                patch_id: args.patch_id,
                kind: proposal.kind,
                target: proposal.target,
                target_path: proposal.target_path.clone(),
                commit_sha: commit_sha.clone(),
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;

    Ok(ScopePatchActionResult::Approved { commit_sha })
}

pub(super) async fn reject_scope_patch(
    rpc: &InProcessRpc,
    args: RejectScopePatchArgs,
) -> RpcResult<ScopePatchActionResult> {
    let repo = resolve_repo(rpc, args.repo_id).await?;
    let repo_path = PathBuf::from(&repo.local_path);
    let queue = match load_queue(&repo_path) {
        Ok(q) => q,
        Err(QueueError::Missing { .. }) => {
            return resolved_or_not_found(&repo_path, args.patch_id);
        }
        Err(e) => return Err(queue_err(e)),
    };
    let mut queue = queue;
    let proposal = match queue.remove(args.patch_id) {
        Some(p) => p,
        None => return resolved_or_not_found(&repo_path, args.patch_id),
    };
    write_queue(&repo_path, &queue).map_err(queue_err)?;

    let subject = format!("scope-patch reject: {}", short_rationale(&proposal));
    let body = build_reject_body(&proposal, args.reason.as_deref());
    let full = combine_message(&subject, &body);

    commit_paths(
        &repo_path,
        &full,
        &[repo_path.join("DOCS").join("SCOPE-PROPOSED.md")],
    )
    .map_err(|e| RpcError::Internal(format!("git commit: {e}")))?;

    let commit_sha =
        head_sha(&repo_path).map_err(|e| RpcError::Internal(format!("git rev-parse HEAD: {e}")))?;

    let stage_id = synthetic_stage_id();
    let review_id = synthetic_review_id();
    rpc.bus
        .publish(
            None,
            Some(stage_id),
            None,
            Event::ScopePatchRejected {
                stage_id,
                review_id,
                patch_id: args.patch_id,
                kind: proposal.kind,
                target: proposal.target,
                target_path: proposal.target_path.clone(),
                commit_sha: commit_sha.clone(),
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;

    Ok(ScopePatchActionResult::Rejected { commit_sha })
}

pub(super) async fn edit_scope_patch(
    rpc: &InProcessRpc,
    args: EditScopePatchArgs,
) -> RpcResult<ScopePatchActionResult> {
    let repo = resolve_repo(rpc, args.repo_id).await?;
    let repo_path = PathBuf::from(&repo.local_path);
    let queue = match load_queue(&repo_path) {
        Ok(q) => q,
        Err(QueueError::Missing { .. }) => {
            return resolved_or_not_found(&repo_path, args.patch_id);
        }
        Err(e) => return Err(queue_err(e)),
    };
    if queue.find(args.patch_id).is_none() {
        return resolved_or_not_found(&repo_path, args.patch_id);
    }

    let parsed = parse_queue(&format!("# Proposed scope patches\n\n{}", args.rendered))
        .map_err(|e| RpcError::InvalidArgument(format!("rendered buffer is not parseable: {e}")))?;
    let new_proposal = parsed.proposals.into_iter().next().ok_or_else(|| {
        RpcError::InvalidArgument("rendered buffer contains no proposal block".into())
    })?;
    if new_proposal.id != args.patch_id {
        return Err(RpcError::InvalidArgument(format!(
            "rendered buffer changed the proposal id from `{}` to `{}`",
            args.patch_id, new_proposal.id
        )));
    }

    let mut queue = queue;
    if !queue.replace(new_proposal) {
        return Err(RpcError::Internal(format!(
            "proposal `{}` vanished from queue during edit",
            args.patch_id
        )));
    }
    write_queue(&repo_path, &queue).map_err(queue_err)?;
    Ok(ScopePatchActionResult::Edited)
}

/// Undo a previously-applied approval commit. The 10-second post-
/// approval undo toast in the UI patch inbox is the only call site;
/// see decision OQ#3 ("audit trail records both the approval and the
/// undo") for why this produces a new revert commit instead of a
/// `git reset`. No event is emitted — the runtime cannot reconstruct
/// the `ScopePatchProposed` payload from a SHA alone, and the existing
/// `ScopePatchApproved` envelope on the bus is the resolution record.
pub(super) async fn revert_scope_patch(
    rpc: &InProcessRpc,
    args: RevertScopePatchArgs,
) -> RpcResult<RevertScopePatchResult> {
    let repo = resolve_repo(rpc, args.repo_id).await?;
    let repo_path = PathBuf::from(&repo.local_path);
    let commit_sha = git_revert(&repo_path, &args.commit_sha)
        .map_err(|e| RpcError::Internal(format!("git revert: {e}")))?;
    Ok(RevertScopePatchResult { commit_sha })
}

async fn resolve_repo(rpc: &InProcessRpc, repo_id: RepoId) -> RpcResult<Repo> {
    rpc.store
        .get_repo(repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {repo_id}")))
}

fn queue_err(e: QueueError) -> RpcError {
    match e {
        QueueError::Missing { path } => {
            RpcError::NotFound(format!("DOCS/SCOPE-PROPOSED.md not found at {path}"))
        }
        QueueError::Io { path, source } => RpcError::Internal(format!("read {path}: {source}")),
        QueueError::Parse(msg) => RpcError::Internal(format!("parse queue: {msg}")),
    }
}

/// Translate "patch not in queue" into either an `AlreadyResolved`
/// response (when `git log` shows a prior approval/rejection) or a
/// plain `NotFound`. Centralised so all three RPCs agree on the
/// distinction.
fn resolved_or_not_found(repo: &Path, patch_id: ScopePatchId) -> RpcResult<ScopePatchActionResult> {
    match find_patch_resolution(repo, &patch_id.to_string())
        .map_err(|e| RpcError::Internal(format!("git log: {e}")))?
    {
        Some(PriorPatchResolution::Approved { commit_sha }) => {
            Ok(ScopePatchActionResult::AlreadyResolved {
                resolution: ScopePatchResolution::Approved,
                commit_sha: Some(commit_sha),
            })
        }
        Some(PriorPatchResolution::Rejected { commit_sha }) => {
            Ok(ScopePatchActionResult::AlreadyResolved {
                resolution: ScopePatchResolution::Rejected,
                commit_sha: Some(commit_sha),
            })
        }
        None => Err(RpcError::NotFound(format!(
            "no proposed patch with id `{patch_id}`"
        ))),
    }
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

fn build_approve_body(p: &Proposal, includes: &[String]) -> Vec<String> {
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
            lines.push(format!("    - {inc}"));
        }
    }
    lines.push(String::new());
    lines.push("Rationale:".into());
    for r in p.rationale.lines() {
        lines.push(format!("  {r}"));
    }
    lines.push(String::new());
    lines.push(UI_TRAILER.into());
    lines
}

fn build_reject_body(p: &Proposal, reason: Option<&str>) -> Vec<String> {
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
    lines.push(String::new());
    lines.push(UI_TRAILER.into());
    lines
}

fn combine_message(subject: &str, body: &[String]) -> String {
    let mut out = String::from(subject);
    if !body.is_empty() {
        out.push_str("\n\n");
        out.push_str(&body.join("\n"));
    }
    out
}

/// The on-disk queue format does not preserve the originating
/// `stage_id` / `review_id` from `ScopePatchProposed` (the queue only
/// stores `evidence_stage_id` for `Loosen` patches). The resolution
/// event still carries both fields for shape-parity with
/// `ScopePatchProposed`; the runtime emits fresh ids when the queue is
/// the only available source. Cross-window subscribers key off
/// `patch_id`, which is the durable identity across the propose →
/// resolve transition.
fn synthetic_stage_id() -> codeless_types::StageId {
    codeless_types::StageId::new()
}

fn synthetic_review_id() -> codeless_types::ReviewId {
    codeless_types::ReviewId::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{Repo, RepoId};
    use std::process::Command;
    use tempfile::TempDir;

    fn git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "ui-test@example.com"]);
        git(p, &["config", "user.name", "UI Test"]);
        git(p, &["commit", "--allow-empty", "-q", "-m", "root"]);
        dir
    }

    async fn seed_repo_row(rpc: &InProcessRpc, local_path: &Path) -> RepoId {
        let id = RepoId::new();
        let now = now_ms();
        let repo = Repo {
            id,
            name: "test".into(),
            clone_url: "x".into(),
            default_branch: "main".into(),
            local_path: local_path.to_string_lossy().into_owned(),
            git_auth: codeless_types::GitAuth::Ssh {
                key_path: String::new(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: now,
            updated_at: now,
        };
        rpc.store.insert_repo(&repo).await.unwrap();
        id
    }

    fn write_queue_file(repo: &Path, patch_id: ScopePatchId, target_rel: &str) {
        let docs = repo.join("DOCS");
        std::fs::create_dir_all(&docs).unwrap();
        let body = format!(
            "# Proposed scope patches\n\n## {patch_id}\n\n\
             - kind: tighten\n\
             - target: claude-md\n\
             - target-path: {target_rel}\n\
             - has_predicate: false\n\n\
             ### Rationale\n\nR4 should explicitly auto-FAIL stages that touch other files\n\n\
             ### Body\n\nappend the sentence to R4\n",
        );
        std::fs::write(docs.join("SCOPE-PROPOSED.md"), body).unwrap();
    }

    #[tokio::test]
    async fn approve_emits_event_and_returns_sha() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let patch_id = ScopePatchId::new();
        std::fs::write(dir.path().join("CLAUDE.md"), "x\n").unwrap();
        write_queue_file(dir.path(), patch_id, "CLAUDE.md");

        let mut stream = rpc
            .bus
            .subscribe_since(crate::event_bus::SubscribeFilter::All, None)
            .await
            .unwrap();

        let res = approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id,
                message: None,
                include: vec![],
            },
        )
        .await
        .unwrap();
        let sha = match res {
            ScopePatchActionResult::Approved { commit_sha } => commit_sha,
            other => panic!("expected Approved, got {other:?}"),
        };
        assert_eq!(sha.len(), 40);

        // Event arrives on the bus.
        use futures_util::StreamExt;
        let env = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match env.event {
            Event::ScopePatchApproved {
                patch_id: pid,
                commit_sha,
                ..
            } => {
                assert_eq!(pid, patch_id);
                assert_eq!(commit_sha, sha);
            }
            other => panic!("expected ScopePatchApproved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn approve_twice_returns_already_resolved() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let patch_id = ScopePatchId::new();
        std::fs::write(dir.path().join("CLAUDE.md"), "x\n").unwrap();
        write_queue_file(dir.path(), patch_id, "CLAUDE.md");

        let first = approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id,
                message: None,
                include: vec![],
            },
        )
        .await
        .unwrap();
        let first_sha = match first {
            ScopePatchActionResult::Approved { commit_sha } => commit_sha,
            other => panic!("expected Approved, got {other:?}"),
        };

        let second = approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id,
                message: None,
                include: vec![],
            },
        )
        .await
        .unwrap();
        match second {
            ScopePatchActionResult::AlreadyResolved {
                resolution: ScopePatchResolution::Approved,
                commit_sha: Some(sha),
            } => assert_eq!(sha, first_sha),
            other => panic!("expected AlreadyResolved/Approved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reject_then_approve_reports_rejected() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let patch_id = ScopePatchId::new();
        std::fs::write(dir.path().join("CLAUDE.md"), "x\n").unwrap();
        write_queue_file(dir.path(), patch_id, "CLAUDE.md");

        reject_scope_patch(
            &rpc,
            RejectScopePatchArgs {
                repo_id,
                patch_id,
                reason: Some("overconstrains".into()),
            },
        )
        .await
        .unwrap();

        let again = approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id,
                message: None,
                include: vec![],
            },
        )
        .await
        .unwrap();
        match again {
            ScopePatchActionResult::AlreadyResolved {
                resolution: ScopePatchResolution::Rejected,
                commit_sha: Some(_),
            } => {}
            other => panic!("expected AlreadyResolved/Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn approve_unknown_patch_returns_not_found() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let res = approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id: ScopePatchId::new(),
                message: None,
                include: vec![],
            },
        )
        .await;
        match res {
            Err(RpcError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_replaces_proposal_body() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let patch_id = ScopePatchId::new();
        write_queue_file(dir.path(), patch_id, "CLAUDE.md");

        let rendered = format!(
            "## {patch_id}\n\n\
             - kind: tighten\n\
             - target: claude-md\n\
             - target-path: CLAUDE.md\n\
             - has_predicate: false\n\n\
             ### Rationale\n\nReplaced rationale\n\n\
             ### Body\n\nReplaced body\n",
        );
        let res = edit_scope_patch(
            &rpc,
            EditScopePatchArgs {
                repo_id,
                patch_id,
                rendered,
            },
        )
        .await
        .unwrap();
        assert!(matches!(res, ScopePatchActionResult::Edited));

        let q = load_queue(dir.path()).unwrap();
        let updated = q.find(patch_id).unwrap();
        assert_eq!(updated.rationale, "Replaced rationale");
        assert_eq!(updated.body, "Replaced body");
    }

    #[tokio::test]
    async fn approve_commit_body_carries_ui_trailer() {
        let rpc = InProcessRpc::new().await.unwrap();
        let dir = init_repo();
        let repo_id = seed_repo_row(&rpc, dir.path()).await;
        let patch_id = ScopePatchId::new();
        std::fs::write(dir.path().join("CLAUDE.md"), "x\n").unwrap();
        write_queue_file(dir.path(), patch_id, "CLAUDE.md");

        approve_scope_patch(
            &rpc,
            ApproveScopePatchArgs {
                repo_id,
                patch_id,
                message: None,
                include: vec![],
            },
        )
        .await
        .unwrap();

        let out = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["log", "-1", "--format=%B"])
            .output()
            .unwrap();
        let body = String::from_utf8_lossy(&out.stdout);
        assert!(body.contains(UI_TRAILER), "missing trailer: {body}");
        assert!(
            body.contains(&format!("Approved scope patch {patch_id}")),
            "missing approval marker: {body}"
        );
    }
}
