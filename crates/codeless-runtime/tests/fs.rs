use std::sync::Arc;

use codeless_adapters_host::HostFs;
use codeless_rpc::{
    FsReadDirArgs, FsReadFileArgs, FsStatArgs, FsWriteFileArgs, RpcError, RpcServer,
};
use codeless_runtime::rpc::InProcessRpc;
use tempfile::tempdir;

#[tokio::test]
async fn fs_methods_unconfigured_return_internal() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .fs_read_dir(FsReadDirArgs {
            path: ".".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
}

#[tokio::test]
async fn fs_write_then_read_through_rpc() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);

    rpc.fs_write_file(FsWriteFileArgs {
        path: "note.md".to_owned(),
        content: "# hello".to_owned(),
    })
    .await
    .unwrap();

    let got = rpc
        .fs_read_file(FsReadFileArgs {
            path: "note.md".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(got.content, "# hello");
}

#[tokio::test]
async fn fs_traversal_maps_to_invalid_argument() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);

    let err = rpc
        .fs_read_file(FsReadFileArgs {
            path: "../etc/passwd".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test]
async fn fs_stat_missing_returns_none_kind() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);

    let stat = rpc
        .fs_stat(FsStatArgs {
            path: "no-such-file".to_owned(),
        })
        .await
        .unwrap();
    assert!(stat.kind.is_none());
}

#[tokio::test]
async fn fs_read_dir_returns_sorted_entries() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("b.txt"), "y").unwrap();
    std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
    let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);

    let res = rpc
        .fs_read_dir(FsReadDirArgs {
            path: ".".to_owned(),
        })
        .await
        .unwrap();
    let names: Vec<_> = res.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt"]);
}
