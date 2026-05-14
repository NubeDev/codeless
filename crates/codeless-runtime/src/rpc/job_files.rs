use codeless_adapters_host::{GitCommitError, GitDiffError, commit_paths, diff_against};
use codeless_rpc::{
    DeleteJobFileArgs, JobDiffArgs, JobDiffFile, JobDiffResult, JobFileEntry, ListJobFilesArgs,
    ListJobFilesResult, ReadJobFileArgs, ReadJobFileResult, RpcError, RpcResult,
    UpdateJobTemplateArgs, UpdateJobTemplateResult, WriteHandoverArgs, WriteHandoverResult,
    WriteJobFileArgs, WriteJobFileResult,
};
use codeless_types::Event;

use super::InProcessRpc;
use crate::job_dir::{
    self, FilenameError, JobLayout, directory_path, flat_yaml_path, sanitise_filename,
    template_yaml_path,
};
use crate::template::JobTemplate;
use crate::time::now_ms;

/// Resolve a `job_id` to the repo's on-disk path and the job's
/// directory name. Template jobs use the template's `name` field;
/// prompt-only jobs fall back to `job-<id>`.
pub(super) async fn resolve_repo_and_template_name(
    rpc: &InProcessRpc,
    job_id: codeless_types::JobId,
) -> RpcResult<(std::path::PathBuf, String)> {
    let job = rpc
        .store
        .get_job(job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {job_id}")))?;
    let name = match job.template_yaml.as_ref() {
        Some(yaml) => {
            let template = JobTemplate::parse_yaml(yaml).map_err(|e| {
                RpcError::InvalidArgument(format!("job {job_id} template parse: {e}"))
            })?;
            template.name
        }
        None => format!("job-{job_id}"),
    };
    let repo = rpc
        .store
        .get_repo(job.repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
    Ok((std::path::PathBuf::from(repo.local_path), name))
}

pub(super) async fn list_job_files(
    rpc: &InProcessRpc,
    args: ListJobFilesArgs,
) -> RpcResult<ListJobFilesResult> {
    let (repo_path, name) = resolve_repo_and_template_name(rpc, args.job_id).await?;
    let layout = job_dir::resolve(&repo_path, &name);
    let mut entries: Vec<JobFileEntry> = Vec::new();
    let mut directory_path_str: Option<String> = None;

    if matches!(layout, JobLayout::Directory | JobLayout::FlatPreferred) {
        let dir = directory_path(&repo_path, &name);
        directory_path_str = Some(dir.to_string_lossy().into_owned());
        let read_dir = std::fs::read_dir(&dir)
            .map_err(|e| RpcError::Internal(format!("read job dir {}: {e}", dir.display())))?;
        let mut files: Vec<std::path::PathBuf> = read_dir
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();

        let mut tpl: Option<JobFileEntry> = None;
        for path in files {
            let base = match path.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let lower = base.to_ascii_lowercase();
            let entry = JobFileEntry {
                name: base.clone(),
                is_template: lower == "template.yaml",
                is_scope: lower == "scope.md",
                is_workflow: lower == "workflow.md",
            };
            if entry.is_template {
                tpl = Some(entry);
            } else {
                entries.push(entry);
            }
        }
        if let Some(t) = tpl {
            entries.insert(0, t);
        }
    }

    Ok(ListJobFilesResult {
        entries,
        layout: layout.wire_name().to_string(),
        directory_path: directory_path_str,
    })
}

pub(super) async fn read_job_file(
    rpc: &InProcessRpc,
    args: ReadJobFileArgs,
) -> RpcResult<ReadJobFileResult> {
    let (repo_path, name) = resolve_repo_and_template_name(rpc, args.job_id).await?;
    let filename = sanitise_filename(&args.filename).map_err(filename_err)?;
    let path = directory_path(&repo_path, &name).join(&filename);
    let content = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            RpcError::NotFound(format!("job file {name}/{filename}"))
        }
        _ => RpcError::Internal(format!("read {}: {e}", path.display())),
    })?;
    Ok(ReadJobFileResult { content })
}

pub(super) async fn write_job_file(
    rpc: &InProcessRpc,
    args: WriteJobFileArgs,
) -> RpcResult<WriteJobFileResult> {
    let (repo_path, name) = resolve_repo_and_template_name(rpc, args.job_id).await?;
    let filename = sanitise_filename(&args.filename).map_err(filename_err)?;

    let layout = job_dir::resolve(&repo_path, &name);
    if matches!(layout, JobLayout::Flat | JobLayout::FlatPreferred) {
        migrate_flat_to_directory(&repo_path, &name)?;
    }

    let dir = directory_path(&repo_path, &name);
    std::fs::create_dir_all(&dir)
        .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", dir.display())))?;
    let path = dir.join(&filename);
    std::fs::write(&path, &args.content)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", path.display())))?;
    commit_paths(
        &repo_path,
        &format!("update job-file: {name}/{filename}"),
        std::slice::from_ref(&path),
    )
    .map_err(git_commit_err)?;

    rpc.bus
        .publish(
            Some(args.job_id),
            None,
            None,
            Event::JobFileUpdated {
                job_id: args.job_id,
                filename: filename.clone(),
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;

    Ok(WriteJobFileResult { name: filename })
}

pub(super) async fn delete_job_file(
    rpc: &InProcessRpc,
    args: DeleteJobFileArgs,
) -> RpcResult<()> {
    let (repo_path, name) = resolve_repo_and_template_name(rpc, args.job_id).await?;
    let filename = sanitise_filename(&args.filename).map_err(filename_err)?;
    let path = directory_path(&repo_path, &name).join(&filename);
    if !path.exists() {
        return Err(RpcError::NotFound(format!("job file {name}/{filename}")));
    }
    std::fs::remove_file(&path)
        .map_err(|e| RpcError::Internal(format!("delete {}: {e}", path.display())))?;
    commit_paths(
        &repo_path,
        &format!("delete job-file: {name}/{filename}"),
        &[path],
    )
    .map_err(git_commit_err)?;
    rpc.bus
        .publish(
            Some(args.job_id),
            None,
            None,
            Event::JobFileUpdated {
                job_id: args.job_id,
                filename,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn update_job_template(
    rpc: &InProcessRpc,
    args: UpdateJobTemplateArgs,
) -> RpcResult<UpdateJobTemplateResult> {
    let parsed = JobTemplate::parse_yaml(&args.template_yaml)
        .map_err(|e| RpcError::InvalidArgument(format!("template parse: {e}")))?;

    let mut job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let prev_name = match job.template_yaml.as_deref() {
        Some(prev) => match JobTemplate::parse_yaml(prev) {
            Ok(tpl) => tpl.name,
            Err(_) => parsed.name.clone(),
        },
        None => parsed.name.clone(),
    };
    if prev_name != parsed.name {
        return Err(RpcError::Conflict(format!(
            "rename refused: spec name is `{prev_name}`, cannot become `{}`. Submit a fresh job to rename.",
            parsed.name,
        )));
    }

    let repo = rpc
        .store
        .get_repo(job.repo_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
    let repo_path = std::path::PathBuf::from(repo.local_path);

    let layout = job_dir::resolve(&repo_path, &parsed.name);
    if matches!(layout, JobLayout::Flat | JobLayout::FlatPreferred) {
        migrate_flat_to_directory(&repo_path, &parsed.name)?;
    }

    let tpl_path = template_yaml_path(&repo_path, &parsed.name);
    if let Some(parent) = tpl_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            RpcError::Internal(format!("create job dir {}: {e}", parent.display()))
        })?;
    }
    std::fs::write(&tpl_path, &args.template_yaml)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl_path.display())))?;
    commit_paths(
        &repo_path,
        &format!("update template: {}", parsed.name),
        std::slice::from_ref(&tpl_path),
    )
    .map_err(git_commit_err)?;

    job.template_yaml = Some(args.template_yaml);
    if !rpc.store.update_job(&job).await.map_err(super::db_err)? {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }

    rpc.bus
        .publish(
            Some(args.job_id),
            None,
            None,
            Event::JobTemplateUpdated {
                job_id: args.job_id,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;

    Ok(UpdateJobTemplateResult { name: parsed.name })
}

pub(super) async fn write_handover(
    rpc: &InProcessRpc,
    args: WriteHandoverArgs,
) -> RpcResult<WriteHandoverResult> {
    let job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let worktree = job.worktree_path.as_deref().ok_or_else(|| {
        RpcError::Conflict(format!(
            "job {} has no worktree yet; the runner must run before a handover can be seeded",
            args.job_id
        ))
    })?;
    let path = crate::handover::write_handover(
        std::path::Path::new(worktree),
        args.job_id,
        &args.handover,
    )
    .await
    .map_err(|e| RpcError::Internal(format!("write handover: {e}")))?;
    Ok(WriteHandoverResult {
        path: path.to_string_lossy().into_owned(),
    })
}

pub(super) async fn job_diff(rpc: &InProcessRpc, args: JobDiffArgs) -> RpcResult<JobDiffResult> {
    let Some(job) = rpc.store.get_job(args.job_id).await.map_err(super::db_err)? else {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    };
    let Some(repo) = rpc.store.get_repo(job.repo_id).await.map_err(super::db_err)? else {
        return Err(RpcError::NotFound(format!("repo {}", job.repo_id)));
    };
    // `Job.branch` is the canonical branch name written back by the
    // runtime on worktree provisioning. Falls back for legacy rows.
    let head = if job.branch.trim().is_empty() {
        format!("codeless/job-{}", job.id)
    } else {
        job.branch.clone()
    };
    let base = repo.default_branch.clone();
    let repo_path = std::path::PathBuf::from(&repo.local_path);
    // Wrap with `spawn_blocking` so a slow git invocation does not
    // stall the tokio reactor.
    let head_clone = head.clone();
    let base_clone = base.clone();
    let files =
        tokio::task::spawn_blocking(move || diff_against(&repo_path, &base_clone, &head_clone))
            .await
            .map_err(|e| RpcError::Internal(format!("git diff join: {e}")))?
            .map_err(diff_err)?;
    let files = files
        .into_iter()
        .map(|f| JobDiffFile {
            path: f.path,
            status: f.status,
            additions: f.additions,
            deletions: f.deletions,
            is_binary: f.is_binary,
            patch: f.patch,
        })
        .collect();
    Ok(JobDiffResult { base, head, files })
}

/// Seed a fresh job directory at `<repo>/.codeless/jobs/<name>/` with
/// `template.yaml`, `SCOPE.md`, and `WORKFLOW.md`, and commit them in
/// a single commit. Called from `submit_job` so the spec exists on disk
/// from the moment the row appears in the dashboard.
///
/// Refuses (`Conflict`) if the directory already exists. `template.yaml`
/// parse errors surface as `InvalidArgument` so the UI can show the
/// line/column inline.
pub(super) fn seed_job_directory(
    repo_local_path: &str,
    template_yaml: &str,
) -> Result<(), RpcError> {
    let parsed = JobTemplate::parse_yaml(template_yaml)
        .map_err(|e| RpcError::InvalidArgument(format!("template parse: {e}")))?;

    let repo_path = std::path::PathBuf::from(repo_local_path);
    let dir = directory_path(&repo_path, &parsed.name);
    if dir.exists() {
        return Err(RpcError::Conflict(format!(
            "a job named `{}` already exists at {}; pick a different name",
            parsed.name,
            dir.display(),
        )));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", dir.display())))?;

    let tpl_path = template_yaml_path(&repo_path, &parsed.name);
    std::fs::write(&tpl_path, template_yaml)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl_path.display())))?;

    let scope_path = dir.join("SCOPE.md");
    std::fs::write(&scope_path, SCOPE_PRESET)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", scope_path.display())))?;

    let workflow_path = dir.join("WORKFLOW.md");
    std::fs::write(&workflow_path, WORKFLOW_PRESET)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", workflow_path.display())))?;

    commit_paths(
        &repo_path,
        &format!("scaffold job: {}", parsed.name),
        &[tpl_path, scope_path, workflow_path],
    )
    .map_err(git_commit_err)?;

    Ok(())
}

/// Promote a legacy flat `<name>.yaml` to the directory layout. Two
/// separate commits — write the new file first, then delete the flat
/// YAML — so `git log` records the move as two atomic steps and a crash
/// between them leaves both files on disk, which `JobLayout` surfaces as
/// `FlatPreferred` and a retry resolves.
fn migrate_flat_to_directory(repo: &std::path::Path, name: &str) -> RpcResult<()> {
    let flat = flat_yaml_path(repo, name);
    let tpl = template_yaml_path(repo, name);
    let body = std::fs::read_to_string(&flat)
        .map_err(|e| RpcError::Internal(format!("read flat {}: {e}", flat.display())))?;
    if let Some(parent) = tpl.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", parent.display())))?;
    }
    std::fs::write(&tpl, &body)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl.display())))?;
    commit_paths(
        repo,
        &format!("migrate template: {name} → directory layout"),
        &[tpl],
    )
    .map_err(git_commit_err)?;

    std::fs::remove_file(&flat)
        .map_err(|e| RpcError::Internal(format!("remove flat {}: {e}", flat.display())))?;
    commit_paths(
        repo,
        &format!("migrate template: {name} (remove flat YAML)"),
        &[flat],
    )
    .map_err(git_commit_err)?;
    Ok(())
}

fn filename_err(e: FilenameError) -> RpcError {
    match e {
        FilenameError::PathTraversal => {
            RpcError::InvalidArgument("filename contains path traversal".to_owned())
        }
        FilenameError::Dotfile => RpcError::InvalidArgument("dotfiles are not allowed".to_owned()),
        FilenameError::ReservedTemplateYaml => {
            RpcError::InvalidArgument("template.yaml is reserved; use the spec editor".to_owned())
        }
        FilenameError::Empty => RpcError::InvalidArgument("filename is empty".to_owned()),
    }
}

fn git_commit_err(e: GitCommitError) -> RpcError {
    RpcError::Internal(format!("git: {e}"))
}

/// Translate `GitDiffError` into wire errors. Missing-ref cases map to
/// `NotFound` so the UI's files-changed tab can render an empty state
/// rather than an error toast — the common cause is "job ran without a
/// worktree provisioned" and that's expected, not exceptional.
fn diff_err(e: GitDiffError) -> RpcError {
    match e {
        GitDiffError::BaseMissing(b) => RpcError::NotFound(format!("base ref {b}")),
        GitDiffError::HeadMissing(h) => RpcError::NotFound(format!("head ref {h}")),
        GitDiffError::Io(err) => RpcError::Internal(format!("git io: {err}")),
        GitDiffError::GitFailed { op, status, stderr } => {
            RpcError::Internal(format!("git {op} failed ({status}): {stderr}"))
        }
    }
}

const SCOPE_PRESET: &str = "# Scope\n\n\
What this job is for. Replace this with what success looks like, what\n\
is out of scope, the constraints, and the deliverables.\n";

const WORKFLOW_PRESET: &str = "# Workflow\n\n\
How the agent should drive the work. Replace this with how to sequence\n\
the stages, what to verify between them, and what counts as done.\n";
