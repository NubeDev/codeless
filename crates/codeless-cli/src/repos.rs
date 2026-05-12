//! `codeless repos <verb>` — the first dual-mode CLI verb. Picks
//! `HttpRpcClient` when `--core` is set, otherwise falls back to the
//! in-process runtime. The verb stays minimal on purpose: it exists
//! to prove the hosted-mode round-trip; richer repo management is a
//! later phase that builds on this scaffolding.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use codeless_client::{HttpRpcClient, HttpRpcClientConfig};
use codeless_rpc::{ListReposResult, RpcServer};

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// Print every repo registered in the core, one ULID + name per
    /// line. Local-mode reads from `--db`; hosted-mode hits
    /// `GET /rpc/list_repos` on `--core`.
    List,
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
        let rpc = build_rpc(core, token, db).await?;
        match verb {
            Verb::List => list(rpc.as_ref()).await,
        }
    })
}

async fn build_rpc(
    core: Option<String>,
    token: Option<String>,
    db: Option<PathBuf>,
) -> Result<Arc<dyn RpcServer>> {
    if let Some(base_url) = core {
        // Hosted-mode: every `--core` invocation builds a fresh
        // `reqwest::Client` via `HttpRpcClient::new`; the connection
        // pool inside is per-instance, which is fine for a one-shot
        // CLI invocation. Long-running consumers should hold onto
        // the client.
        let cfg = HttpRpcClientConfig {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        };
        let client = HttpRpcClient::new(cfg).map_err(|e| anyhow!("http client: {e}"))?;
        return Ok(Arc::new(client));
    }
    if token.is_some() {
        return Err(anyhow!("--token only meaningful with --core"));
    }
    let rpc = rpc_open::open(db.as_deref()).await?;
    Ok(Arc::new(rpc))
}

async fn list(rpc: &dyn RpcServer) -> Result<ExitCode> {
    let ListReposResult { repos } = rpc
        .list_repos()
        .await
        .map_err(|e| anyhow!("list_repos: {e}"))?;
    if repos.is_empty() {
        eprintln!("(no repos)");
        return Ok(ExitCode::SUCCESS);
    }
    for repo in &repos {
        println!("{}\t{}", repo.id, repo.name);
    }
    Ok(ExitCode::SUCCESS)
}
