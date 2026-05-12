//! Shared helper for opening the in-process runtime against either
//! the user-specified SQLite file (`--db`) or a fresh in-memory pool.
//! Centralised so the `run` and `review` subcommands cannot drift on
//! how the pool is configured.

use std::path::Path;

use anyhow::{anyhow, Result};
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
