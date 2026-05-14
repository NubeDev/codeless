use codeless_rpc::{AddRepoArgs, ListReposResult, RemoveRepoArgs, RpcError, RpcResult};
use codeless_types::{Event, Repo, RepoId};

use super::InProcessRpc;
use crate::time::now_ms;

pub(super) async fn add_repo(rpc: &InProcessRpc, args: AddRepoArgs) -> RpcResult<Repo> {
    let now = now_ms();
    let repo = Repo {
        id: RepoId::new(),
        name: args.name,
        clone_url: args.clone_url,
        default_branch: args.default_branch,
        local_path: args.local_path,
        git_auth: args.git_auth,
        concurrency_cap: args.concurrency_cap,
        default_runner: args.default_runner,
        created_at: now,
        updated_at: now,
    };
    rpc.store.insert_repo(&repo).await.map_err(super::db_err)?;
    rpc.bus
        .publish(None, None, None, Event::RepoAdded { repo_id: repo.id }, now)
        .await
        .map_err(super::db_err)?;
    Ok(repo)
}

pub(super) async fn remove_repo(rpc: &InProcessRpc, args: RemoveRepoArgs) -> RpcResult<()> {
    let removed = rpc
        .store
        .remove_repo(args.repo_id)
        .await
        .map_err(super::db_err)?;
    if !removed {
        return Err(RpcError::NotFound(format!("repo {}", args.repo_id)));
    }
    rpc.bus
        .publish(
            None,
            None,
            None,
            Event::RepoRemoved {
                repo_id: args.repo_id,
            },
            now_ms(),
        )
        .await
        .map_err(super::db_err)?;
    Ok(())
}

pub(super) async fn list_repos(rpc: &InProcessRpc) -> RpcResult<ListReposResult> {
    Ok(ListReposResult {
        repos: rpc.store.list_repos().await.map_err(super::db_err)?,
    })
}
