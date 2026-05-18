use serde::{Deserialize, Serialize};

/// Permission tier a tool declares against its persona. Matches the
/// `tier` enum in `tool.wit` (PLUGIN-WASM.md) one-for-one so the
/// builtin and wasm flavours emit identical manifests.
///
/// Semantics:
///
/// - [`Tier::Read`] -- no side effects observable outside the
///   request/response. Free to call without a confirmation card.
/// - [`Tier::Write`] -- creates or mutates plugin-owned state
///   (its `<plugin_id>_*` tables, an attachment row, etc.). The
///   Assistant agent loop (PS8) gates these behind an action card.
/// - [`Tier::Destructive`] -- removes data or fires an external
///   irreversible effect. Held to the same gating as `Write` today;
///   carried as a distinct variant so a future per-thread policy can
///   refuse the tier outright without touching the tool source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Read,
    Write,
    Destructive,
}

impl Tier {
    /// Canonical lowercase string -- the form the `plugin.toml`
    /// manifest and the WIT `tier` enum both use.
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::Read => "read",
            Tier::Write => "write",
            Tier::Destructive => "destructive",
        }
    }
}
