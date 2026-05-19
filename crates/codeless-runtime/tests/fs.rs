use std::sync::Arc;

use codeless_adapters_host::HostFs;
use codeless_rpc::{
    AttachWorkspaceArgs, FsCwdArgs, FsReadDirArgs, FsReadFileArgs, FsStatArgs, FsWriteFileArgs,
    RpcError, RpcServer,
};
use codeless_runtime::rpc::InProcessRpc;
use codeless_types::{GitAuth, RepoId};
use tempfile::tempdir;

/// Mint a `repos` row + an `attached_workspaces` attachment pointing
/// at `root`. Returns the freshly-minted `RepoId` so tests can use it
/// in `fs.*` args. The attach goes through the real `add_repo` /
/// `attach_workspace` RPCs so the runtime's mirror into `HostFs`
/// (the allow-list mutation) runs as it would on the live path.
async fn attach(rpc: &InProcessRpc, root: &std::path::Path) -> RepoId {
    // `repos.name` is UNIQUE; mint a per-call suffix so two `attach`
    // helpers in the same test never collide.
    let name = format!("test-{}", RepoId::new());
    let repo = rpc
        .add_repo(codeless_rpc::AddRepoArgs {
            name,
            clone_url: format!("file://{}", root.display()),
            default_branch: "main".into(),
            local_path: root.to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .expect("add_repo");
    rpc.attach_workspace(AttachWorkspaceArgs {
        repo_id: repo.id,
        fs_root_override: None,
    })
    .await
    .expect("attach_workspace");
    repo.id
}

#[tokio::test]
async fn fs_methods_unconfigured_return_internal() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id: RepoId::new(),
            path: ".".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
}

#[tokio::test]
async fn fs_unknown_repo_returns_not_found() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);

    let err = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id: RepoId::new(),
            path: ".".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn fs_write_then_read_through_rpc() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    rpc.fs_write_file(FsWriteFileArgs {
        repo_id,
        path: "note.md".to_owned(),
        content: "# hello".to_owned(),
    })
    .await
    .unwrap();

    let got = rpc
        .fs_read_file(FsReadFileArgs {
            repo_id,
            path: "note.md".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(got.content, "# hello");
}

#[tokio::test]
async fn fs_traversal_maps_to_invalid_argument() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    let err = rpc
        .fs_read_file(FsReadFileArgs {
            repo_id,
            path: "../etc/passwd".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test]
async fn fs_stat_missing_returns_none_kind() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    let stat = rpc
        .fs_stat(FsStatArgs {
            repo_id,
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
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    let res = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id,
            path: ".".to_owned(),
        })
        .await
        .unwrap();
    let names: Vec<_> = res.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt"]);
}

/// Two attached workspaces, each owning their own jail. A read with
/// repo A's `repo_id` must only see A's files even though both roots
/// are registered with the same `HostFs`. This is the regression the
/// workspace-scoping job exists to prevent: a stale tab pointing at A
/// could otherwise enumerate B's filesystem just because B is also
/// attached on the same server.
#[tokio::test]
async fn fs_read_dir_is_jailed_to_the_repo_id() {
    let a = tempdir().unwrap();
    let b = tempdir().unwrap();
    std::fs::write(a.path().join("only-in-a.txt"), "a").unwrap();
    std::fs::write(b.path().join("only-in-b.txt"), "b").unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_a = attach(&rpc, a.path()).await;
    let repo_b = attach(&rpc, b.path()).await;

    let in_a = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id: repo_a,
            path: ".".to_owned(),
        })
        .await
        .unwrap();
    let names_a: Vec<_> = in_a.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names_a, vec!["only-in-a.txt"]);

    let in_b = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id: repo_b,
            path: ".".to_owned(),
        })
        .await
        .unwrap();
    let names_b: Vec<_> = in_b.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names_b, vec!["only-in-b.txt"]);

    // Cross-tab leakage check: an absolute path under B's root passed
    // via A's repo_id must be refused — the jail is per-call.
    let cross = rpc
        .fs_read_file(FsReadFileArgs {
            repo_id: repo_a,
            path: b
                .path()
                .join("only-in-b.txt")
                .to_string_lossy()
                .into_owned(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(cross, RpcError::InvalidArgument(_)),
        "got {cross:?}",
    );
}

#[tokio::test]
async fn fs_cwd_returns_the_repos_canonical_root() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    let cwd = rpc.fs_cwd(FsCwdArgs { repo_id }).await.unwrap();
    let canonical = std::fs::canonicalize(tmp.path()).unwrap();
    assert_eq!(cwd.path, canonical.to_string_lossy());
}

#[tokio::test]
async fn fs_root_for_repo_helper_matches_attached_workspaces() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;

    let root = rpc.fs_root_for_repo(repo_id).await.unwrap();
    let canonical = std::fs::canonicalize(tmp.path()).unwrap();
    assert_eq!(root, canonical);

    let missing = rpc.fs_root_for_repo(RepoId::new()).await.unwrap_err();
    assert!(matches!(missing, RpcError::NotFound(_)), "got {missing:?}");
}

#[tokio::test]
async fn fs_calls_against_detached_repo_id_are_refused() {
    let tmp = tempdir().unwrap();
    let fs = Arc::new(HostFs::empty());
    let rpc = InProcessRpc::new().await.unwrap().with_fs(fs);
    let repo_id = attach(&rpc, tmp.path()).await;
    rpc.detach_workspace(codeless_rpc::DetachWorkspaceArgs {
        repo_id,
        on_running_jobs: codeless_rpc::DetachPolicy::Refuse,
    })
    .await
    .unwrap();

    let err = rpc
        .fs_read_dir(FsReadDirArgs {
            repo_id,
            path: ".".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}
