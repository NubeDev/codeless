//! Cross-plugin MCP contribution checks (Invariant 1 from
//! DOCS/plugins/PLUGIN-MCP.md): every MCP tool has a non-MCP twin.
//!
//! The manifest module already strict-validates the `[contributes.mcp]`
//! block's *shape* (id charset, duplicate ids, HTTP method casing,
//! resource backing). What it deliberately cannot do is verify the
//! parity rule — that requires knowledge the host owns: the
//! `ToolRegistry` of registered codeless tools and the set of REST
//! routes mounted on the server. That knowledge is not in
//! `codeless-tools` either: the REST routes live in `codeless-server`.
//! So the check accepts both as parameters and lives next to the
//! manifest so the substrate's stage-13 / stage-14 split (one place
//! per concern) holds.
//!
//! The check is host-policy, not plugin policy: the doc rule is "the
//! MCP server never gains a code path that bypasses the rest of the
//! public surface", which is enforced by the host at boot, not by
//! the plugin author at write time. A plugin that declares an MCP
//! tool with no twin still parses; loading fails here with a
//! structured error the host (or `codeless plugin info`) can surface.

use super::manifest::{PluginManifest, PluginMcpDispatch};

/// Per-tool view onto a MCP contribution rejection. The host wraps
/// these in its `Failed` outcome (PLUGIN-MCP.md acceptance §2/§3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpParityError {
    /// `dispatch.kind = "tool_call"` named `tool_id` that is not in
    /// the host's `ToolRegistry`. Acceptance §2 of PLUGIN-MCP.md.
    #[error(
        "contributes.mcp.tools[{index}] (`{plugin_id}.{tool_id_local}`): \
         dispatch.tool_call.tool_id `{tool_id}` is not a registered codeless tool"
    )]
    UnknownToolCallTarget {
        index: usize,
        plugin_id: String,
        tool_id_local: String,
        tool_id: String,
    },
    /// `dispatch.kind = "rest_proxy"` named `path` that is not in the
    /// host's mounted REST route set. Acceptance §3 of PLUGIN-MCP.md.
    #[error(
        "contributes.mcp.tools[{index}] (`{plugin_id}.{tool_id_local}`): \
         dispatch.rest_proxy.path `{method} {path}` is not a registered REST route"
    )]
    UnknownRestProxyTarget {
        index: usize,
        plugin_id: String,
        tool_id_local: String,
        method: String,
        path: String,
    },
    /// OQ-MCP-1: `dispatch.kind = "mcp_forward"` parses today but
    /// v0.1 ships no upstream-session lifecycle. The contribution
    /// loads as Failed with the stable structured reason so a
    /// future v0.2 can flip the variant on without manifest
    /// shape-changes.
    #[error(
        "contributes.mcp.tools[{index}] (`{plugin_id}.{tool_id_local}`): \
         mcp_forward not yet supported (OQ-MCP-1; v0.2)"
    )]
    McpForwardNotSupported {
        index: usize,
        plugin_id: String,
        tool_id_local: String,
    },
}

impl McpParityError {
    /// Wire-stable code string for `codeless plugin info` / structured
    /// listings. Stage 14 doesn't ship a JSON projection yet but the
    /// shape is enumerated here so the surface is forward-compatible.
    pub fn code(&self) -> &'static str {
        match self {
            McpParityError::UnknownToolCallTarget { .. } => "mcp-unknown-tool-call-target",
            McpParityError::UnknownRestProxyTarget { .. } => "mcp-unknown-rest-proxy-target",
            McpParityError::McpForwardNotSupported { .. } => "mcp-forward-not-supported",
        }
    }
}

/// Host view onto the parity-check inputs. Two small slice-of-`&str`
/// views are simpler than threading the real `ToolRegistry` /
/// `Router` into `codeless-tools` (which would pull host-only deps
/// the substrate doc rules out). Callers project from their owned
/// state.
#[derive(Debug, Clone, Copy, Default)]
pub struct McpParityCheckInputs<'a> {
    pub registered_tool_ids: &'a [&'a str],
    pub registered_rest_routes: &'a [&'a str],
}

/// Run the parity rule against a plugin manifest. Returns `Ok(())`
/// when every `[[contributes.mcp.tools]]` entry has a real twin. The
/// first failure short-circuits because a plugin with one broken
/// dispatch is treated as broken at the contribution layer -- a
/// half-loaded MCP surface (some tools, some skipped) is the
/// "structural parity" rule's anti-target.
///
/// `mcp_forward` is rejected here, not at manifest parse, because the
/// rejection is host-policy (OQ-MCP-1: v0.2 may flip this on without
/// a manifest shape change). Manifest parse stays "the shape is
/// legal"; the loader is "the host can satisfy it today".
pub fn check_parity(
    manifest: &PluginManifest,
    inputs: McpParityCheckInputs<'_>,
) -> Result<(), McpParityError> {
    let Some(mcp) = manifest.contributes.mcp.as_ref() else {
        return Ok(());
    };
    for (index, t) in mcp.tools.iter().enumerate() {
        let plugin_id = manifest.plugin.id.clone();
        let tool_id_local = t.id.clone();
        match &t.dispatch {
            PluginMcpDispatch::ToolCall { tool_id } => {
                if !inputs.registered_tool_ids.iter().any(|s| *s == tool_id) {
                    return Err(McpParityError::UnknownToolCallTarget {
                        index,
                        plugin_id,
                        tool_id_local,
                        tool_id: tool_id.clone(),
                    });
                }
            }
            PluginMcpDispatch::RestProxy { method, path } => {
                if !inputs.registered_rest_routes.iter().any(|s| *s == path) {
                    return Err(McpParityError::UnknownRestProxyTarget {
                        index,
                        plugin_id,
                        tool_id_local,
                        method: method.clone(),
                        path: path.clone(),
                    });
                }
            }
            PluginMcpDispatch::McpForward {} => {
                return Err(McpParityError::McpForwardNotSupported {
                    index,
                    plugin_id,
                    tool_id_local,
                });
            }
        }
    }
    Ok(())
}

/// Build the namespaced MCP-listing id for a plugin contribution.
/// Centralised here so the listing surface in `codeless-mcp` and the
/// audit field in the same crate produce byte-identical strings -- a
/// drift between the two would break PLUGIN-MCP.md decision-locked
/// item 7 ("Plugin id is a first-class audit field").
pub fn mcp_listing_id(plugin_id: &str, tool_id_local: &str) -> String {
    format!("{plugin_id}.{tool_id_local}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::plugin::manifest::PluginManifest;

    fn manifest_with_mcp(dispatch_toml: &str) -> PluginManifest {
        let text = format!(
            r#"
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
dispatch        = {dispatch_toml}
"#
        );
        PluginManifest::from_str(&text, Some(PathBuf::from("/tmp/notes"))).expect("manifest parses")
    }

    #[test]
    fn tool_call_with_registered_twin_passes() {
        let manifest = manifest_with_mcp(r#"{ kind = "tool_call", tool_id = "notes.append" }"#);
        let inputs = McpParityCheckInputs {
            registered_tool_ids: &["notes.append"],
            registered_rest_routes: &[],
        };
        check_parity(&manifest, inputs).expect("parity passes");
    }

    #[test]
    fn tool_call_with_missing_twin_fails_with_structured_reason() {
        let manifest =
            manifest_with_mcp(r#"{ kind = "tool_call", tool_id = "estimate.does_not_exist" }"#);
        let inputs = McpParityCheckInputs {
            registered_tool_ids: &["notes.append"],
            registered_rest_routes: &[],
        };
        let err = check_parity(&manifest, inputs).unwrap_err();
        assert!(matches!(err, McpParityError::UnknownToolCallTarget { .. }));
        assert_eq!(err.code(), "mcp-unknown-tool-call-target");
    }

    #[test]
    fn rest_proxy_with_missing_route_fails() {
        let manifest = manifest_with_mcp(
            r#"{ kind = "rest_proxy", method = "POST", path = "/api/v1/missing" }"#,
        );
        let inputs = McpParityCheckInputs {
            registered_tool_ids: &[],
            registered_rest_routes: &["/api/v1/notes"],
        };
        let err = check_parity(&manifest, inputs).unwrap_err();
        assert!(matches!(err, McpParityError::UnknownRestProxyTarget { .. }));
    }

    #[test]
    fn mcp_forward_lands_failed_per_oq_mcp_1() {
        let manifest = manifest_with_mcp(r#"{ kind = "mcp_forward" }"#);
        let inputs = McpParityCheckInputs::default();
        let err = check_parity(&manifest, inputs).unwrap_err();
        assert!(matches!(err, McpParityError::McpForwardNotSupported { .. }));
        assert_eq!(err.code(), "mcp-forward-not-supported");
    }

    #[test]
    fn listing_id_is_plugin_dot_local() {
        assert_eq!(
            mcp_listing_id("notes", "notes_append"),
            "notes.notes_append"
        );
    }
}
