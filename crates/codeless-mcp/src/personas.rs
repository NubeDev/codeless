//! Persona lookup behind the MCP prompts surface.
//!
//! Stage 10 publishes every persona row where `use_for_jobs = 1` as
//! an MCP prompt (`AGENT-DECISIONS.md` D3: that flag is the single
//! dimension gating MCP visibility — there is no parallel
//! `expose_via_mcp` column). The handler asks a `PersonaSource` for
//! the filtered list and for a single persona by id; the trait keeps
//! the prompt surface independent of how personas are persisted, so
//! the binary can wire a real `SqliteStore` while tests inject a
//! deterministic fixture without booting sqlx.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use codeless_runtime::{SqliteStore, MIGRATOR};
use codeless_types::Persona;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// How `CodelessMcpHandler` reaches the persona catalogue. Keeps
/// stage-10's surface decoupled from SQLite so a non-DB caller (the
/// stdio MVP binary started without `CODELESS_DB_PATH`) and a
/// fixture-driven test can both supply their own implementation.
#[async_trait]
pub trait PersonaSource: Send + Sync {
    /// All personas with `use_for_jobs = 1`. Order is the source's
    /// natural order; the handler does not re-sort.
    async fn list_for_jobs(&self) -> Vec<Persona>;

    /// Single persona by id, regardless of `use_for_jobs`. The
    /// handler is the layer that gates the flag, because a
    /// `prompts/get` for a persona that has flipped to chat-only
    /// after a `list_prompts` snapshot is a not-found, not a denial.
    async fn get(&self, id: &str) -> Option<Persona>;
}

/// The default source used when the host has not handed the server a
/// real catalogue (the stdio MVP main without `CODELESS_DB_PATH`).
/// Returning empty is correct: there are no personas to expose, and
/// `list_prompts` simply reports an empty list rather than failing.
pub struct EmptyPersonaSource;

#[async_trait]
impl PersonaSource for EmptyPersonaSource {
    async fn list_for_jobs(&self) -> Vec<Persona> {
        Vec::new()
    }

    async fn get(&self, _id: &str) -> Option<Persona> {
        None
    }
}

/// SQLite-backed source reading the `personas` table maintained by
/// the runtime. Read-only — the prompts surface never edits personas.
pub struct SqlitePersonaSource {
    store: Arc<SqliteStore>,
}

impl SqlitePersonaSource {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl PersonaSource for SqlitePersonaSource {
    async fn list_for_jobs(&self) -> Vec<Persona> {
        match self.store.list_personas().await {
            Ok(rows) => rows.into_iter().filter(|p| p.use_for_jobs).collect(),
            Err(err) => {
                // A DB read failure here is non-fatal for the prompt
                // surface; we degrade to "no prompts visible" rather
                // than crashing the MCP connection mid-handshake.
                tracing::warn!(error = %err, "list_personas failed; returning empty MCP prompts");
                Vec::new()
            }
        }
    }

    async fn get(&self, id: &str) -> Option<Persona> {
        match self.store.get_persona(id).await {
            Ok(row) => row,
            Err(err) => {
                tracing::warn!(error = %err, persona_id = id, "get_persona failed");
                None
            }
        }
    }
}

/// Open the runtime's SQLite database for read access and wrap it in
/// a `SqlitePersonaSource`. The file is created if missing and the
/// runtime's migrator runs before any read — same contract as
/// `InProcessRpc::with_file`, kept here so the MCP bin can wire the
/// prompts surface without depending on the in-process RPC type.
pub async fn open_sqlite_persona_source(
    path: &Path,
) -> Result<Arc<dyn PersonaSource>, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await?;
    MIGRATOR.run(&pool).await?;
    let store = Arc::new(SqliteStore::new(pool));
    Ok(Arc::new(SqlitePersonaSource::new(store)))
}
