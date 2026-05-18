//! Plugin MCP contribution surface (DOCS/plugins/PLUGIN-MCP.md
//! § Manifest, § Dispatch path).
//!
//! The MCP server keeps a sibling table of plugin-contributed tools
//! alongside the host's `ToolRegistry`. Each entry knows:
//!
//! - the wire name the MCP client sees (`<plugin_id>.<tool_id_local>`,
//!   from `codeless_tools::plugin::mcp_listing_id`);
//! - the dispatch shape resolved from the manifest -- `tool_call` is
//!   the only kind dispatched in v0.1; `rest_proxy` is structurally
//!   recognised here so a future stage can wire it through the
//!   in-process REST router without re-touching the manifest parser;
//!   `mcp_forward` never lands here because the parity check rejects
//!   it at load (OQ-MCP-1);
//! - the plugin id, used as the audit-event `plugin_id` field (lock
//!   #7 of PLUGIN-MCP.md: "plugin id is a first-class audit field,
//!   not parsed from the tool id at query time").
//!
//! Built at host boot by walking the loaded plugins; immutable for
//! the server's lifetime (same posture as `PluginCatalog` in
//! `codeless-server`).

use codeless_tools::plugin::{LoadedPlugin, McpTier, PluginMcpDispatch};

/// One row in the MCP-contributions table. The host walks the
/// loaded plugins after `PluginRegistry::load_plugin` and after
/// `check_mcp_parity` has run; the dispatch field is the resolved
/// shape so the MCP handler does not re-parse the manifest at
/// `tools/call` time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpContribution {
    /// Plugin owner. First-class so audit and per-plugin off-switch
    /// stay O(1) lookups.
    pub plugin_id: String,
    /// Wire name the MCP listing exposes. Computed once at boot via
    /// `mcp_listing_id(plugin_id, tool_id_local)`.
    pub listing_name: String,
    pub title: String,
    pub tier: McpTier,
    pub dispatch: ResolvedMcpDispatch,
}

/// Resolved dispatch. Carries the same data the manifest's
/// `PluginMcpDispatch` enum did, minus the `mcp_forward` variant
/// (rejected at parity check). Splitting the type at the trust
/// boundary means a future v0.2 that wires `mcp_forward` adds one
/// variant here without re-touching every match arm in stage-14
/// code -- the manifest enum stays the authoritative shape; this
/// enum is the host-policy projection of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedMcpDispatch {
    /// Dispatch through the host's `ToolRegistry`. The handler
    /// looks up `tool_id` and calls it the same way the codeless
    /// agent would (Invariant 1 -- no MCP-only code paths).
    ToolCall { tool_id: String },
    /// Dispatch through the in-process REST router. v0.1 carries
    /// the shape only; the actual proxy invocation is a follow-up.
    /// Surfaced today so `parity_rule_rejects_missing_twin` can
    /// exercise the `path` matching at load time.
    RestProxy { method: String, path: String },
}

/// Build the contribution rows for one loaded plugin. The output is
/// flat (one row per `[[contributes.mcp.tools]]` entry); the host
/// concatenates per-plugin output into the server-wide table.
///
/// `mcp_forward` is *not* produced here -- the parity check is the
/// gate that catches it, so callers should have already short-
/// circuited a plugin whose parity failed. As a defence-in-depth
/// measure, an `mcp_forward` entry that slips past parity is dropped
/// silently rather than panicking; the server still boots, and the
/// missing listing is the operator-visible signal.
pub fn rows_for_loaded_plugin(plugin: &LoadedPlugin) -> Vec<McpContribution> {
    let Some(mcp) = plugin.manifest.contributes.mcp.as_ref() else {
        return Vec::new();
    };
    if !mcp.enabled {
        return Vec::new();
    }
    let plugin_id = plugin.manifest.plugin.id.clone();
    let mut rows = Vec::with_capacity(mcp.tools.len());
    for t in &mcp.tools {
        let listing_name = codeless_tools::plugin::mcp_listing_id(&plugin_id, &t.id);
        let dispatch = match &t.dispatch {
            PluginMcpDispatch::ToolCall { tool_id } => ResolvedMcpDispatch::ToolCall {
                tool_id: tool_id.clone(),
            },
            PluginMcpDispatch::RestProxy { method, path } => ResolvedMcpDispatch::RestProxy {
                method: method.clone(),
                path: path.clone(),
            },
            PluginMcpDispatch::McpForward {} => continue,
        };
        rows.push(McpContribution {
            plugin_id: plugin_id.clone(),
            listing_name,
            title: t.title.clone(),
            tier: t.tier,
            dispatch,
        });
    }
    rows
}

/// Immutable in-memory index keyed by listing name. The handler
/// looks one up per `tools/call`, so the lookup must be O(1); a
/// `Vec` scan would silently regress on a plugin tree of any size.
#[derive(Debug, Clone, Default)]
pub struct McpContributionTable {
    rows: Vec<McpContribution>,
    /// Whether plugin contributions are visible at all. PLUGIN-MCP.md
    /// § Off-switch hierarchy layer 4 ("plugin surface (host
    /// config)"): set to `false` and every plugin tool disappears
    /// from `tools/list`, every `tools/call` against a plugin tool
    /// is rejected. Core MCP tools stay live -- the operator may
    /// trust the codeless team's MCP but not (yet) third-party
    /// plugins.
    enabled: bool,
}

impl McpContributionTable {
    /// Build the table from an iterator of rows. `enabled` defaults
    /// to `true` because a host that built the table explicitly is
    /// opting into plugin MCP visibility; the off-switch is the
    /// post-construction call.
    pub fn from_rows(rows: impl IntoIterator<Item = McpContribution>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            enabled: true,
        }
    }

    /// PLUGIN-MCP.md § Off-switch hierarchy. Set this to `false`
    /// (e.g. via `mcp.plugin_tools_enabled = false` in codeless
    /// config) to hide every plugin contribution from `tools/list`
    /// and reject `tools/call` against them.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn rows(&self) -> &[McpContribution] {
        &self.rows
    }

    /// Iterate over the listings the MCP handler should expose right
    /// now. When the off-switch is engaged this returns an empty
    /// iterator without dropping the rows themselves -- toggling the
    /// switch back on (a future hot-reload knob) restores them
    /// without a registry rebuild.
    pub fn visible_rows(&self) -> impl Iterator<Item = &McpContribution> {
        let enabled = self.enabled;
        self.rows.iter().filter(move |_| enabled)
    }

    /// Look up a contribution by its listing name. Returns `None`
    /// when the table is disabled even if the row exists, so the
    /// `tools/call` handler does not need a second `enabled` check.
    pub fn lookup(&self, listing_name: &str) -> Option<&McpContribution> {
        if !self.enabled {
            return None;
        }
        self.rows.iter().find(|r| r.listing_name == listing_name)
    }

    /// Convenience: every listing name the table currently exposes.
    /// Useful for the handler's `list_tools` collation and for tests.
    pub fn listing_names(&self) -> Vec<&str> {
        if !self.enabled {
            return Vec::new();
        }
        self.rows.iter().map(|r| r.listing_name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codeless_tools::plugin::{LoadedPersona, LoadedPlugin, PluginManifest};

    use super::*;

    fn manifest_with_two_tools() -> PluginManifest {
        let text = r#"
[plugin]
id      = "notes"
version = "0.1.0"
crate   = "codeless-plugin-notes"

[[personas]]
id                          = "notes"
prompt_file                 = "prompts/system.md"
allowed_tools               = ["notes.*"]
default_model_family        = "smart"
default_attachments_policy  = "inline-thread-scoped"

[contributes.mcp]
enabled = true

[[contributes.mcp.tools]]
id              = "notes_append"
title           = "Append a note"
description_md  = "docs/mcp/notes_append.md"
input_schema    = "schemas/notes_append_in.json"
output_schema   = "schemas/notes_append_out.json"
tier            = "write"
dispatch        = { kind = "tool_call", tool_id = "notes.append" }

[[contributes.mcp.tools]]
id              = "notes_render"
title           = "Render a note as PDF"
description_md  = "docs/mcp/notes_render.md"
input_schema    = "schemas/notes_render_in.json"
output_schema   = "schemas/notes_render_out.json"
tier            = "read"
dispatch        = { kind = "rest_proxy", method = "POST", path = "/api/v1/notes/render" }
"#;
        PluginManifest::from_str(text, Some(PathBuf::from("/tmp/notes"))).expect("parses")
    }

    fn fixture_loaded() -> LoadedPlugin {
        LoadedPlugin {
            manifest: manifest_with_two_tools(),
            tool_ids: vec!["notes.append".into()],
            personas: Vec::<LoadedPersona>::new(),
            migrations: Vec::new(),
        }
    }

    #[test]
    fn rows_namespace_listing_under_plugin_id() {
        let rows = rows_for_loaded_plugin(&fixture_loaded());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].listing_name, "notes.notes_append");
        assert_eq!(rows[1].listing_name, "notes.notes_render");
        assert!(matches!(
            rows[0].dispatch,
            ResolvedMcpDispatch::ToolCall { .. }
        ));
        assert!(matches!(
            rows[1].dispatch,
            ResolvedMcpDispatch::RestProxy { .. }
        ));
    }

    #[test]
    fn off_switch_hides_every_row_without_dropping_them() {
        let rows = rows_for_loaded_plugin(&fixture_loaded());
        let table = McpContributionTable::from_rows(rows).with_enabled(false);
        assert!(table.listing_names().is_empty());
        assert!(table.lookup("notes.notes_append").is_none());
        // The rows are still present so a future hot-flip turns them
        // back on without a rebuild. A drop-on-disable would force a
        // full registry walk on every toggle.
        assert_eq!(table.rows().len(), 2);
    }

    #[test]
    fn manifest_level_opt_out_drops_rows_entirely() {
        // PLUGIN-MCP.md § Off-switch: a plugin that wants to ship
        // MCP eventually but not in its first release sets
        // `contributes.mcp.enabled = false`. The host honors it at
        // row-build time, not just at listing time.
        let mut manifest = manifest_with_two_tools();
        manifest.contributes.mcp.as_mut().unwrap().enabled = false;
        let plugin = LoadedPlugin {
            manifest,
            tool_ids: Vec::new(),
            personas: Vec::new(),
            migrations: Vec::new(),
        };
        assert!(rows_for_loaded_plugin(&plugin).is_empty());
    }
}
