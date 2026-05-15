//! `attach_workspace` / `detach_workspace` / `list_workspaces` /
//! `validate_workspace_path` RPCs. The `attached_workspaces` table is
//! the source of truth (R4); methods here read/write rows and surface
//! structured `WorkspaceError` variants so the UI never has to string
//! match on a generic `Conflict`.

use std::path::{Path, PathBuf};

use codeless_rpc::{
    AttachWorkspaceArgs, AttachWorkspaceResult, AttachedWorkspace, DetachPolicy,
    DetachWorkspaceArgs, ListWorkspacesResult, RpcError, RpcResult, ValidateWorkspacePathArgs,
    ValidateWorkspacePathResult, WorkspaceError, WorkspaceProblem,
};
use codeless_types::{Event, JobId, JobStatus, RepoId, UnixMillis};
use sqlx::Row;

use super::InProcessRpc;
use crate::time::now_ms;

pub(super) async fn attach_workspace(
    rpc: &InProcessRpc,
    args: AttachWorkspaceArgs,
) -> RpcResult<AttachWorkspaceResult> {
    let repo = rpc
        .store
        .get_repo(args.repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", args.repo_id)))?;

    // The effective root is the override (when supplied) or the repo's
    // own `local_path`. Both are canonicalised; the override must be a
    // descendant of the repo's `local_path` so attach can't quietly
    // relocate a repo's fs jail outside the registered tree.
    let local_canonical = canonicalise(Path::new(&repo.local_path)).map_err(|_| {
        RpcError::Workspace(WorkspaceError::PathRejected {
            problems: vec![WorkspaceProblem::NotADirectory],
        })
    })?;
    let display_input = args
        .fs_root_override
        .clone()
        .unwrap_or_else(|| repo.local_path.clone());
    let canonical_path = match args.fs_root_override.as_deref() {
        Some(raw) => {
            let candidate = canonicalise(Path::new(raw)).map_err(|_| {
                RpcError::Workspace(WorkspaceError::PathRejected {
                    problems: vec![WorkspaceProblem::NotADirectory],
                })
            })?;
            let mut problems = problems_for_path(&candidate);
            if !candidate.starts_with(&local_canonical) {
                problems.push(WorkspaceProblem::InsideAnotherWorkspace {
                    other_root: local_canonical.display().to_string(),
                });
            }
            if !problems.is_empty() {
                return Err(RpcError::Workspace(WorkspaceError::PathRejected {
                    problems,
                }));
            }
            candidate
        }
        None => local_canonical.clone(),
    };
    let canonical = canonical_path.to_string_lossy().into_owned();

    // The unique index on `fs_root_canonical` is the source of truth
    // for "already attached" — query it directly so a stale row from a
    // boot upsert surfaces as `AlreadyAttached` (with the existing
    // repo) rather than a foreign-key crash on insert.
    let existing: Option<String> =
        sqlx::query_scalar("SELECT repo_id FROM attached_workspaces WHERE fs_root_canonical = ?")
            .bind(&canonical)
            .fetch_optional(rpc.pool())
            .await
            .map_err(super::db_err)?;
    if let Some(existing_repo) = existing {
        let existing_repo_id = existing_repo
            .parse::<RepoId>()
            .map_err(|e| RpcError::Internal(format!("stored repo_id: {e}")))?;
        return Err(RpcError::Workspace(WorkspaceError::AlreadyAttached {
            repo_id: existing_repo_id,
            fs_root: canonical,
        }));
    }

    let now = now_ms();
    sqlx::query(
        "INSERT INTO attached_workspaces \
         (repo_id, fs_root_canonical, fs_root_display, attached_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(args.repo_id.to_string())
    .bind(&canonical)
    .bind(&display_input)
    .bind(now.0)
    .execute(rpc.pool())
    .await
    .map_err(super::db_err)?;

    // Mirror the row into the host adapter's allowed-roots list so the
    // `fs.*` surface accepts paths under this workspace immediately —
    // without this, an attach would persist but the UI's first
    // `fs_read_dir` would `PermissionDenied`. The adapter is optional
    // (tests + headless modes skip `with_fs`); when absent the DB row
    // is still the source of truth and a later boot will rehydrate.
    if let Some(fs) = rpc.fs.as_ref() {
        if let Err(e) = fs.add_root(&canonical_path) {
            tracing::warn!(error = %e, path = %canonical, "attach: add_root failed");
        }
    }

    // Repos are 1:1 with attachments via the primary key — keep the
    // event payload consistent with `RepoUpdated` so subscribers that
    // already redraw the sidebar on repo changes pick this up too.
    rpc.bus
        .publish(
            None,
            None,
            None,
            Event::RepoUpdated {
                repo_id: args.repo_id,
            },
            now,
        )
        .await
        .map_err(super::db_err)?;

    Ok(AttachWorkspaceResult {
        workspace: AttachedWorkspace {
            repo_id: args.repo_id,
            repo_name: repo.name,
            fs_root: canonical,
            attached_at: now,
            default_runner: repo.default_runner,
        },
    })
}

pub(super) async fn detach_workspace(
    rpc: &InProcessRpc,
    args: DetachWorkspaceArgs,
) -> RpcResult<()> {
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM attached_workspaces WHERE repo_id = ? LIMIT 1")
            .bind(args.repo_id.to_string())
            .fetch_optional(rpc.pool())
            .await
            .map_err(super::db_err)?;
    if exists.is_none() {
        return Err(RpcError::Workspace(WorkspaceError::NotAttached));
    }

    // `Refuse` is the safe default: surface the running jobs so the UI
    // can prompt before destruction. `Stop` cancels them first; the
    // current implementation drives `stop_job` through the existing
    // RPC path, which publishes the `JobStopped` envelopes the
    // dashboard listens for. `LeaveRunning` skips both — runners keep
    // their per-job worktree handles regardless of this row.
    let running = running_jobs_for_repo(rpc, args.repo_id).await?;
    match args.on_running_jobs {
        DetachPolicy::Refuse => {
            if !running.is_empty() {
                return Err(RpcError::Workspace(WorkspaceError::RunningJobs {
                    jobs: running,
                }));
            }
        }
        DetachPolicy::Stop => {
            for job_id in &running {
                if let Err(err) =
                    super::jobs::stop_job(rpc, codeless_rpc::StopJobArgs { job_id: *job_id }).await
                {
                    // Already-terminal races (the job finished between
                    // our scan and the stop call) are fine; anything
                    // else propagates so the UI can show the failure.
                    if !matches!(err, RpcError::Conflict(_) | RpcError::NotFound(_)) {
                        return Err(err);
                    }
                }
            }
        }
        DetachPolicy::LeaveRunning => {}
    }

    let removed_root: Option<String> =
        sqlx::query_scalar("SELECT fs_root_canonical FROM attached_workspaces WHERE repo_id = ?")
            .bind(args.repo_id.to_string())
            .fetch_optional(rpc.pool())
            .await
            .map_err(super::db_err)?;

    sqlx::query("DELETE FROM attached_workspaces WHERE repo_id = ?")
        .bind(args.repo_id.to_string())
        .execute(rpc.pool())
        .await
        .map_err(super::db_err)?;

    // Pull the corresponding entry out of the host adapter so a
    // subsequent `fs.*` call against the detached path surfaces as
    // `PermissionDenied`, matching the doc's "subsequent fs_* calls
    // under that path return PermissionDenied (not Internal)" rule.
    if let (Some(fs), Some(canonical)) = (rpc.fs.as_ref(), removed_root.as_deref()) {
        fs.remove_root(Path::new(canonical));
    }

    rpc.bus
        .publish(
            None,
            None,
            None,
            Event::RepoUpdated {
                repo_id: args.repo_id,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn list_workspaces(rpc: &InProcessRpc) -> RpcResult<ListWorkspacesResult> {
    let rows = sqlx::query(
        "SELECT aw.repo_id, aw.fs_root_canonical, aw.attached_at, \
                r.name, r.default_runner \
         FROM attached_workspaces aw \
         JOIN repos r ON r.id = aw.repo_id \
         ORDER BY aw.attached_at ASC, aw.repo_id ASC",
    )
    .fetch_all(rpc.pool())
    .await
    .map_err(super::db_err)?;

    let mut workspaces = Vec::with_capacity(rows.len());
    for row in rows {
        let repo_id: String = row.get("repo_id");
        let repo_id = repo_id
            .parse::<RepoId>()
            .map_err(|e| RpcError::Internal(format!("stored repo_id: {e}")))?;
        workspaces.push(AttachedWorkspace {
            repo_id,
            repo_name: row.get("name"),
            fs_root: row.get("fs_root_canonical"),
            attached_at: UnixMillis(row.get::<i64, _>("attached_at")),
            default_runner: row.get("default_runner"),
        });
    }
    Ok(ListWorkspacesResult { workspaces })
}

pub(super) async fn validate_workspace_path(
    rpc: &InProcessRpc,
    args: ValidateWorkspacePathArgs,
) -> RpcResult<ValidateWorkspacePathResult> {
    let raw = Path::new(&args.path);
    let canonical_path = canonicalise(raw).ok();
    let canonical_string = canonical_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let mut problems = Vec::new();

    let metadata = canonical_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok());

    let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
    if !is_dir {
        problems.push(WorkspaceProblem::NotADirectory);
    }

    let readable = canonical_path
        .as_ref()
        .is_some_and(|p| std::fs::read_dir(p).is_ok());
    if is_dir && !readable {
        problems.push(WorkspaceProblem::NotReadable);
    }
    let writable = canonical_path.as_ref().is_some_and(|p| is_path_writable(p));
    if is_dir && !writable {
        problems.push(WorkspaceProblem::NotWritable);
    }

    let is_git_repo = canonical_path
        .as_ref()
        .is_some_and(|p| p.join(".git").exists());
    if is_dir && !is_git_repo {
        problems.push(WorkspaceProblem::NotAGitRepo);
    }

    if let Some(path) = canonical_path.as_ref() {
        if is_system_path(path) {
            problems.push(WorkspaceProblem::SystemPath);
        }
    }

    let already_attached = if let Some(canonical) = canonical_string.as_deref() {
        sqlx::query_scalar::<_, String>(
            "SELECT repo_id FROM attached_workspaces WHERE fs_root_canonical = ?",
        )
        .bind(canonical)
        .fetch_optional(rpc.pool())
        .await
        .map_err(super::db_err)?
        .is_some()
    } else {
        false
    };
    // `already_attached` is a property the picker renders as a hint;
    // it does not push a `WorkspaceProblem` because re-attaching is
    // not the same shape of refusal as a system path.

    let default_branch = canonical_path.as_deref().and_then(detect_default_branch);

    Ok(ValidateWorkspacePathResult {
        canonical: canonical_string,
        is_dir,
        is_git_repo,
        default_branch,
        already_attached,
        readable,
        writable,
        problems,
    })
}

fn canonicalise(raw: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(raw)
}

fn problems_for_path(path: &Path) -> Vec<WorkspaceProblem> {
    let mut out = Vec::new();
    let meta = std::fs::metadata(path);
    match meta {
        Ok(m) if m.is_dir() => {}
        _ => out.push(WorkspaceProblem::NotADirectory),
    }
    if is_system_path(path) {
        out.push(WorkspaceProblem::SystemPath);
    }
    out
}

fn is_system_path(path: &Path) -> bool {
    matches!(
        path.to_str(),
        Some("/") | Some("/etc") | Some("/usr") | Some("/bin") | Some("/sbin") | Some("/var")
    ) || path.ends_with(".ssh")
        || std::env::var_os("HOME").is_some_and(|home| Path::new(&home) == path)
}

fn is_path_writable(path: &Path) -> bool {
    // POSIX `access(2)` is racey but matches what a picker can promise
    // — the actual write will go through `HostFs` which re-checks at
    // call time. A failing tempfile probe would touch the disk; the
    // metadata-only path is good enough for the picker hint.
    match std::fs::metadata(path) {
        Ok(m) => !m.permissions().readonly(),
        Err(_) => false,
    }
}

fn detect_default_branch(path: &Path) -> Option<String> {
    // Read `.git/HEAD` directly so this stage stays clear of
    // `tokio::process` (R1). The wrapper at `codeless-adapters-host`
    // will later replace this with a `git rev-parse` so detached HEAD
    // and packed refs surface correctly.
    let head = std::fs::read_to_string(path.join(".git").join("HEAD")).ok()?;
    let trimmed = head.trim();
    trimmed
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_owned())
}

async fn running_jobs_for_repo(rpc: &InProcessRpc, repo_id: RepoId) -> RpcResult<Vec<JobId>> {
    let jobs = rpc
        .store
        .list_jobs(Some(repo_id))
        .await
        .map_err(super::db_err)?;
    Ok(jobs
        .into_iter()
        .filter(|j| {
            matches!(
                j.status,
                JobStatus::Running | JobStatus::Queued | JobStatus::AwaitingReview
            )
        })
        .map(|j| j.id)
        .collect())
}
