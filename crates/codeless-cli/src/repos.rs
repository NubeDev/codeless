//! `codeless repos <verb>` — the first dual-mode CLI verb. Picks
//! `HttpRpcClient` when `--core` is set, otherwise falls back to the
//! in-process runtime. The verb stays minimal on purpose: it exists
//! to prove the hosted-mode round-trip; richer repo management is a
//! later phase that builds on this scaffolding.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{AddRepoArgs, ListReposResult, RemoveRepoArgs, RpcServer};
use codeless_types::{GitAuth, RepoId};

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// Print every repo registered in the core. Default output is
    /// one ULID + name per line; `--json` switches to a single JSON
    /// array for shell-pipeline consumers. Local-mode reads from
    /// `--db`; hosted-mode hits `GET /rpc/list_repos` on `--core`.
    List(ListArgs),
    /// Register a new repo with the core. Picks `git_auth: token`
    /// against the supplied env var by default — the only auth
    /// shape the runtime currently understands end-to-end. SSH /
    /// GitHub App auth come back as flags when the runner wiring
    /// catches up.
    Add(AddArgs),
    /// Remove a repo by ULID. Errors with `NotFound` if the id is
    /// unknown.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Emit a single JSON array instead of one row per line.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Display name. Typically the repo slug (e.g. `nube-io/codeless`).
    pub name: String,
    /// Clone URL (https / ssh / file://). The runner adapter resolves
    /// this against the chosen `--auth-env`.
    #[arg(long)]
    pub clone_url: String,
    /// Local path the worktree manager checks out into. Must exist
    /// (or be creatable) on the host running the core, not the
    /// machine running the CLI.
    #[arg(long)]
    pub local_path: String,
    /// Default branch for new jobs that don't override it.
    #[arg(long, default_value = "main")]
    pub default_branch: String,
    /// Env var name on the host that holds the bearer token used by
    /// the runner to push branches back. Default matches the
    /// `GITHUB_TOKEN` convention.
    #[arg(long, default_value = "GITHUB_TOKEN")]
    pub auth_env: String,
    /// Per-repo concurrency cap. `None` (the default) inherits the
    /// global cap.
    #[arg(long)]
    pub concurrency_cap: Option<u32>,
    /// Default runner this repo's jobs use unless overridden.
    /// `mock | claude | anthropic`.
    #[arg(long)]
    pub default_runner: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    pub repo_id: String,
}

pub fn handle(
    verb: Verb,
    core: Option<String>,
    token: Option<String>,
    db: Option<PathBuf>,
) -> Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let rpc = rpc_open::build_dual_mode(core, token, db).await?;
        match verb {
            Verb::List(args) => list(rpc.as_ref(), args).await,
            Verb::Add(args) => add(rpc.as_ref(), args).await,
            Verb::Remove(args) => remove(rpc.as_ref(), args).await,
        }
    })
}

async fn list(rpc: &dyn RpcServer, args: ListArgs) -> Result<ExitCode> {
    let ListReposResult { repos } = rpc
        .list_repos()
        .await
        .map_err(|e| anyhow!("list_repos: {e}"))?;
    if args.json {
        println!("{}", serde_json::to_string(&repos)?);
        return Ok(ExitCode::SUCCESS);
    }
    if repos.is_empty() {
        eprintln!("(no repos)");
        return Ok(ExitCode::SUCCESS);
    }
    for repo in &repos {
        println!("{}\t{}", repo.id, repo.name);
    }
    Ok(ExitCode::SUCCESS)
}

async fn add(rpc: &dyn RpcServer, args: AddArgs) -> Result<ExitCode> {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: args.name,
            clone_url: args.clone_url,
            default_branch: args.default_branch,
            local_path: args.local_path,
            git_auth: GitAuth::Token {
                env_var: args.auth_env,
            },
            concurrency_cap: args.concurrency_cap,
            default_runner: args.default_runner,
        })
        .await
        .map_err(|e| anyhow!("add_repo: {e}"))?;
    println!("{}", repo.id);
    Ok(ExitCode::SUCCESS)
}

async fn remove(rpc: &dyn RpcServer, args: RemoveArgs) -> Result<ExitCode> {
    let repo_id = RepoId::from_str(&args.repo_id)
        .map_err(|e| anyhow!("invalid repo id {:?}: {e}", args.repo_id))?;
    rpc.remove_repo(RemoveRepoArgs { repo_id })
        .await
        .map_err(|e| anyhow!("remove_repo: {e}"))?;
    println!("removed {repo_id}");
    Ok(ExitCode::SUCCESS)
}
