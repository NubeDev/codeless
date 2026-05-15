//! WORKSPACE-ATTACH milestone 1, stage 5 exit criterion:
//! `attach_workspace -> list_workspaces -> detach_workspace` round-trips
//! through `InProcessRpc`, and the structured `WorkspaceError` variants
//! ride back through the typed `RpcError::Workspace` channel so callers
//! branch on the variant rather than string-matching a generic
//! `Conflict`.

use codeless_rpc::{
    AddRepoArgs, AttachWorkspaceArgs, DetachPolicy, DetachWorkspaceArgs, RpcError, RpcServer,
    ValidateWorkspacePathArgs, WorkspaceError, WorkspaceProblem,
};
use codeless_runtime::InProcessRpc;
use codeless_types::GitAuth;
use tempfile::tempdir;

async fn fresh_rpc() -> InProcessRpc {
    InProcessRpc::new().await.expect("open runtime")
}

fn add_repo_args(name: &str, local_path: &str) -> AddRepoArgs {
    AddRepoArgs {
        name: name.into(),
        clone_url: String::new(),
        default_branch: "main".into(),
        local_path: local_path.into(),
        git_auth: GitAuth::Token {
            env_var: String::new(),
        },
        concurrency_cap: None,
        default_runner: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_list_detach_round_trip() {
    let rpc = fresh_rpc().await;
    let dir = tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();

    let repo = rpc
        .add_repo(add_repo_args("alpha", canonical.to_str().unwrap()))
        .await
        .unwrap();

    let attached = rpc
        .attach_workspace(AttachWorkspaceArgs {
            repo_id: repo.id,
            fs_root_override: None,
        })
        .await
        .unwrap();
    assert_eq!(attached.workspace.repo_id, repo.id);
    assert_eq!(attached.workspace.repo_name, "alpha");
    assert_eq!(attached.workspace.fs_root, canonical.to_string_lossy());

    let listed = rpc.list_workspaces().await.unwrap();
    assert_eq!(listed.workspaces.len(), 1);
    assert_eq!(listed.workspaces[0].repo_id, repo.id);
    assert_eq!(listed.workspaces[0].fs_root, canonical.to_string_lossy());

    rpc.detach_workspace(DetachWorkspaceArgs {
        repo_id: repo.id,
        on_running_jobs: DetachPolicy::Refuse,
    })
    .await
    .unwrap();

    let after = rpc.list_workspaces().await.unwrap();
    assert!(
        after.workspaces.is_empty(),
        "detach must remove the workspace row: {after:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn double_attach_surfaces_structured_already_attached() {
    let rpc = fresh_rpc().await;
    let dir = tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();

    let repo = rpc
        .add_repo(add_repo_args("beta", canonical.to_str().unwrap()))
        .await
        .unwrap();
    rpc.attach_workspace(AttachWorkspaceArgs {
        repo_id: repo.id,
        fs_root_override: None,
    })
    .await
    .unwrap();

    let err = rpc
        .attach_workspace(AttachWorkspaceArgs {
            repo_id: repo.id,
            fs_root_override: None,
        })
        .await
        .expect_err("second attach must conflict");
    match err {
        RpcError::Workspace(WorkspaceError::AlreadyAttached { repo_id, fs_root }) => {
            assert_eq!(repo_id, repo.id);
            assert_eq!(fs_root, canonical.to_string_lossy());
        }
        other => panic!("expected AlreadyAttached, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_unknown_repo_surfaces_not_attached() {
    let rpc = fresh_rpc().await;
    let dir = tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let repo = rpc
        .add_repo(add_repo_args("gamma", canonical.to_str().unwrap()))
        .await
        .unwrap();

    let err = rpc
        .detach_workspace(DetachWorkspaceArgs {
            repo_id: repo.id,
            on_running_jobs: DetachPolicy::Refuse,
        })
        .await
        .expect_err("detach without prior attach must fail");
    assert!(
        matches!(err, RpcError::Workspace(WorkspaceError::NotAttached)),
        "got {err:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_flags_system_path_and_non_git() {
    let rpc = fresh_rpc().await;
    let dir = tempdir().unwrap();
    // Plain temp dir: exists, is a directory, but is not a git repo.
    let result = rpc
        .validate_workspace_path(ValidateWorkspacePathArgs {
            path: dir.path().to_string_lossy().into_owned(),
        })
        .await
        .unwrap();
    assert!(result.is_dir);
    assert!(!result.is_git_repo);
    assert!(result.canonical.is_some());
    assert!(result
        .problems
        .iter()
        .any(|p| matches!(p, WorkspaceProblem::NotAGitRepo)));

    // System path: refused regardless of contents.
    let sys = rpc
        .validate_workspace_path(ValidateWorkspacePathArgs {
            path: "/etc".into(),
        })
        .await
        .unwrap();
    assert!(sys
        .problems
        .iter()
        .any(|p| matches!(p, WorkspaceProblem::SystemPath)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validate_reports_already_attached() {
    let rpc = fresh_rpc().await;
    let dir = tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let repo = rpc
        .add_repo(add_repo_args("delta", canonical.to_str().unwrap()))
        .await
        .unwrap();
    rpc.attach_workspace(AttachWorkspaceArgs {
        repo_id: repo.id,
        fs_root_override: None,
    })
    .await
    .unwrap();

    let result = rpc
        .validate_workspace_path(ValidateWorkspacePathArgs {
            path: canonical.to_string_lossy().into_owned(),
        })
        .await
        .unwrap();
    assert!(result.already_attached);
}
