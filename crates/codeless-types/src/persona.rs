use serde::{Deserialize, Serialize};

use crate::time::UnixMillis;

/// A persona row from the `personas` SQLite table (migration
/// `0011_personas.sql`). Personas unify the three previous meanings of
/// "agent": they shape the runner system prompt (job-submit + per-stage
/// override) and they cap which subagents the chat panel may spawn.
///
/// The wire shape mirrors the SQL columns one-for-one so the
/// `RpcClient` surface (added in this stage) can replace the
/// `ai-agents` KV store as the UI's source of truth. The KV store
/// becomes a cache mirroring this record.
///
/// `id` is the lookup key the stage YAML and the chat-side picker both
/// quote. Built-in rows use `builtin:<slug>`; user rows use whatever id
/// the UI mints. The runtime never invents ids — it is whatever the
/// caller passed to `upsert_persona`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Free-form icon identifier the UI maps to a glyph
    /// (`coder` / `architect` / `reviewer` / `security` / `designer` /
    /// `spark`). The runtime stores it verbatim.
    pub icon: String,
    pub instructions: String,
    /// Single dimension gating the job-submit picker AND the MCP
    /// prompt exposure (D3). The runtime exposes a persona as an MCP
    /// prompt iff this flag is set; no parallel `expose_via_mcp`
    /// field on purpose.
    pub use_for_jobs: bool,
    /// Runner-catalogue-specific model id (`claude-opus-4-7`,
    /// `gpt-5.x`, …). `None` means "no preference, use the runner
    /// default".
    pub default_model: Option<String>,
    /// Subagent ids this persona may spawn. The registry already caps
    /// each subagent's tool set to the read-only registry set; this
    /// list narrows further but cannot widen. An empty vec means "no
    /// subagents spawnable".
    pub allowed_subagents: Vec<String>,
    /// Snippet ids the chat panel composes into the system prompt.
    /// Chat-only for MVP (D4); the column exists so a future runtime
    /// change does not need a migration.
    pub default_snippets: Vec<String>,
    /// `true` for the five seeded rows. Built-in rows are not
    /// deletable through `delete_persona`; the rule is enforced at
    /// the RPC layer so the column does not have to grow a CHECK
    /// constraint when a future stage relaxes it.
    pub built_in: bool,
    pub created_at: UnixMillis,
    pub updated_at: UnixMillis,
}
