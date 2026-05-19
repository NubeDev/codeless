use std::path::PathBuf;

use codeless_adapters_host::FsError;
use codeless_rpc::{
    FsCreateDirArgs, FsCreateFileArgs, FsCwdArgs, FsCwdResult, FsDeleteArgs, FsMoveArgs,
    FsReadDirArgs, FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult,
    FsWriteFileArgs, RpcError, RpcResult,
};
use codeless_types::RepoId;

use super::InProcessRpc;

/// Resolve `repo_id` to the attached workspace's `fs_root_canonical`.
/// Returns `NotFound` for an unknown or detached `repo_id` so the UI
/// can render a clear "workspace not attached" rather than a generic
/// `Internal` error. The host adapter is consulted as a
/// defence-in-depth check — the DB row is the source of truth, but if
/// the adapter's allow-list does not include this root the workspace
/// is considered detached even when the row still exists.
pub(crate) async fn fs_root_for_repo(rpc: &InProcessRpc, repo_id: RepoId) -> RpcResult<PathBuf> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT fs_root_canonical FROM attached_workspaces WHERE repo_id = ?")
            .bind(repo_id.to_string())
            .fetch_optional(rpc.pool())
            .await
            .map_err(super::db_err)?;
    let canonical =
        row.ok_or_else(|| RpcError::NotFound(format!("workspace not attached: {repo_id}")))?;
    let path = PathBuf::from(&canonical);
    if let Some(fs) = rpc.fs.as_ref() {
        if !fs.root_is_registered(&path) {
            return Err(RpcError::NotFound(format!(
                "workspace not attached: {repo_id}"
            )));
        }
    }
    Ok(path)
}

pub(super) async fn fs_read_dir(
    rpc: &InProcessRpc,
    args: FsReadDirArgs,
) -> RpcResult<FsReadDirResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    let entries = fs.read_dir_in(&jail, &args.path).await.map_err(fs_err)?;
    Ok(FsReadDirResult { entries })
}

pub(super) async fn fs_read_file(
    rpc: &InProcessRpc,
    args: FsReadFileArgs,
) -> RpcResult<FsReadFileResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    let content = fs.read_file_in(&jail, &args.path).await.map_err(fs_err)?;
    Ok(FsReadFileResult { content })
}

pub(super) async fn fs_write_file(rpc: &InProcessRpc, args: FsWriteFileArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    fs.write_file_in(&jail, &args.path, &args.content)
        .await
        .map_err(fs_err)?;
    Ok(())
}

pub(super) async fn fs_stat(rpc: &InProcessRpc, args: FsStatArgs) -> RpcResult<FsStatResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    let entry = fs.stat_in(&jail, &args.path).await.map_err(fs_err)?;
    Ok(match entry {
        Some((kind, size, mtime)) => FsStatResult {
            kind: Some(kind),
            size,
            mtime,
        },
        None => FsStatResult {
            kind: None,
            size: None,
            mtime: None,
        },
    })
}

pub(super) async fn fs_cwd(rpc: &InProcessRpc, args: FsCwdArgs) -> RpcResult<FsCwdResult> {
    // The adapter is consulted only for the registration check inside
    // `fs_root_for_repo`; `fs_cwd` itself just echoes the DB-resolved
    // canonical path. The early `fs_not_configured` mirrors the rest
    // of the surface so a runtime built without `with_fs` returns
    // `Internal` from every `fs_*` method.
    let _ = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    Ok(FsCwdResult {
        path: jail.to_string_lossy().into_owned(),
    })
}

pub(super) async fn fs_create_file(rpc: &InProcessRpc, args: FsCreateFileArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    fs.create_file_in(&jail, &args.path, args.content.as_deref(), args.overwrite)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_create_dir(rpc: &InProcessRpc, args: FsCreateDirArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    fs.create_dir_in(&jail, &args.path, args.recursive)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_move(rpc: &InProcessRpc, args: FsMoveArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    fs.rename_in(&jail, &args.from, &args.to, args.overwrite)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_delete(rpc: &InProcessRpc, args: FsDeleteArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let jail = fs_root_for_repo(rpc, args.repo_id).await?;
    fs.delete_in(&jail, &args.path, args.recursive)
        .await
        .map_err(fs_err)
}

fn fs_not_configured() -> RpcError {
    RpcError::Internal("fs.* not available: runtime has no filesystem root configured".to_owned())
}

/// Map host-side `FsError` to wire `RpcError`. Path-escape is
/// `InvalidArgument` because the caller supplied the path; IO errors
/// map to `Internal` (typically permission/disk).
pub(super) fn fs_err(e: FsError) -> RpcError {
    match e {
        FsError::Escape(p) => RpcError::InvalidArgument(format!("path escapes root: {p}")),
        // No `RpcError::PermissionDenied` variant exists on the wire
        // yet (adding one is a breaking enum change), so surface the
        // refusal through `InvalidArgument` with the "permission
        // denied" prefix that the UI's empty-state copy is keyed off.
        FsError::PermissionDenied(p) => {
            RpcError::InvalidArgument(format!("permission denied: {p}"))
        }
        FsError::NotUtf8(p) => RpcError::InvalidArgument(format!("not a utf-8 text file: {p}")),
        FsError::BadRoot(p) => {
            RpcError::Internal(format!("fs root misconfigured: {}", p.display()))
        }
        FsError::Io(err) if err.kind() == std::io::ErrorKind::NotFound => {
            RpcError::NotFound(err.to_string())
        }
        FsError::Io(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            RpcError::Conflict(err.to_string())
        }
        FsError::Io(err) => RpcError::Internal(format!("fs io: {err}")),
    }
}
