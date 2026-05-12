//! Shared helper for opening the in-process runtime against either
//! the user-specified SQLite file (`--db`) or a fresh in-memory pool.
//! Centralised so the `run` and `review` subcommands cannot drift on
//! how the pool is configured.
//!
//! `build_dual_mode` extends this for verbs that work over both the
//! local-mode (`InProcessRpc` against `--db`) and hosted-mode
//! (`HttpRpcClient` against `--core`) transports — `repos` and
//! `tail` are the current consumers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use codeless_client::{HttpRpcClient, HttpRpcClientConfig};
use codeless_rpc::RpcServer;
use codeless_runtime::InProcessRpc;

pub async fn open(db: Option<&Path>) -> Result<InProcessRpc> {
    match db {
        None => InProcessRpc::new()
            .await
            .map_err(|e| anyhow!("init runtime: {e}")),
        Some(path) => InProcessRpc::with_file(path)
            .await
            .map_err(|e| anyhow!("open {}: {e}", path.display())),
    }
}

/// Pick an `RpcServer` impl based on the global `--core` / `--token`
/// flags. When `core` is `Some`, builds an `HttpRpcClient`; otherwise
/// opens the in-process runtime against `db`. `--token` without
/// `--core` is rejected so a misuse can't silently fall back to
/// local mode with an unused token.
pub async fn build_dual_mode(
    core: Option<String>,
    token: Option<String>,
    db: Option<PathBuf>,
) -> Result<Arc<dyn RpcServer>> {
    if let Some(base_url) = core {
        let cfg = HttpRpcClientConfig {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        };
        let client = HttpRpcClient::new(cfg).map_err(|e| anyhow!("http client: {e}"))?;
        return Ok(Arc::new(client));
    }
    if token.is_some() {
        bail!("--token only meaningful with --core");
    }
    let rpc = open(db.as_deref()).await?;
    Ok(Arc::new(rpc))
}
