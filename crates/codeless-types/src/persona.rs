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
    /// Substrate-doc allowed-tools list
    /// (`DOCS/PLUGIN-SUBSTRATE.md` items 3 + 5). Each entry is either
    /// a literal tool id (`fs.read`) or a dotted-prefix glob ending in
    /// `.*` (`estimate.*`); no shell globbing, no regex. The matcher
    /// in `codeless_types::allowed_tools` is the single source of
    /// truth -- plugin manifests (item 6) reject any other syntax at
    /// load time. Empty vec is "no MCP tools granted on this
    /// persona", which is the safe default for built-ins that do not
    /// opt in.
    pub allowed_tools: Vec<String>,
    /// Codeless-side model family alias (`fast` / `smart` /
    /// `reasoning`) the runner resolves to a concrete provider model
    /// at call time. Plugins must not hardcode provider model ids --
    /// the mapping lives in codeless config and changes when models
    /// do. `None` means "no preference, use the runner default" and
    /// is a real value distinct from any named alias.
    pub default_model_family: Option<String>,
    /// How thread attachments are surfaced into the prompt at
    /// agent-call time. The substrate doc example
    /// `inline-thread-scoped` is the only value the MVP runner reads;
    /// the column is plain TEXT (no enum) so a future policy
    /// (e.g. `referenced-only`, `vector-indexed`) ships without a
    /// migration.
    pub default_attachments_policy: String,
    /// `true` for seeded rows. Built-in rows are not deletable through
    /// `delete_persona`; the rule is enforced at the RPC layer so the
    /// column does not have to grow a CHECK constraint when a future
    /// stage relaxes it.
    pub built_in: bool,
    pub created_at: UnixMillis,
    pub updated_at: UnixMillis,
}
