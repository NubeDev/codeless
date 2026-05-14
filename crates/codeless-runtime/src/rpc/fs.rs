use codeless_adapters_host::FsError;
use codeless_rpc::{
    FsCreateDirArgs, FsCreateFileArgs, FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs,
    FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs,
    RpcError, RpcResult,
};

use super::InProcessRpc;

pub(super) async fn fs_read_dir(
    rpc: &InProcessRpc,
    args: FsReadDirArgs,
) -> RpcResult<FsReadDirResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let entries = fs.read_dir(&args.path).await.map_err(fs_err)?;
    Ok(FsReadDirResult { entries })
}

pub(super) async fn fs_read_file(
    rpc: &InProcessRpc,
    args: FsReadFileArgs,
) -> RpcResult<FsReadFileResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let content = fs.read_file(&args.path).await.map_err(fs_err)?;
    Ok(FsReadFileResult { content })
}

pub(super) async fn fs_write_file(rpc: &InProcessRpc, args: FsWriteFileArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    fs.write_file(&args.path, &args.content)
        .await
        .map_err(fs_err)?;
    Ok(())
}

pub(super) async fn fs_stat(rpc: &InProcessRpc, args: FsStatArgs) -> RpcResult<FsStatResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    let entry = fs.stat(&args.path).await.map_err(fs_err)?;
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

pub(super) async fn fs_cwd(rpc: &InProcessRpc) -> RpcResult<FsCwdResult> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    Ok(FsCwdResult {
        path: fs.root().to_string_lossy().into_owned(),
    })
}

pub(super) async fn fs_create_file(rpc: &InProcessRpc, args: FsCreateFileArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    fs.create_file(&args.path, args.content.as_deref(), args.overwrite)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_create_dir(rpc: &InProcessRpc, args: FsCreateDirArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    fs.create_dir(&args.path, args.recursive)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_move(rpc: &InProcessRpc, args: FsMoveArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    fs.rename(&args.from, &args.to, args.overwrite)
        .await
        .map_err(fs_err)
}

pub(super) async fn fs_delete(rpc: &InProcessRpc, args: FsDeleteArgs) -> RpcResult<()> {
    let fs = rpc.fs.as_ref().ok_or_else(fs_not_configured)?;
    fs.delete(&args.path, args.recursive).await.map_err(fs_err)
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
