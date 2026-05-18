//! Plugin-substrate-runtimes stage 14 acceptance — exercises the
//! MCP-contributions surface declared in DOCS/plugins/PLUGIN-MCP.md.
//!
//! The three named tests pin the load-bearing contracts:
//!
//! - `tool_call_dispatch_round_trip`: a manifest-declared
//!   `[contributes.mcp]` tool, dispatched via `tool_call`, reaches the
//!   plugin's registered codeless tool through the same
//!   `ToolRegistry::get` path the codeless agent uses (Invariant 1 of
//!   PLUGIN-MCP.md -- no MCP-only code paths). The audit row carries
//!   `plugin_id = "notes"` and `dispatch = "tool_call"` per the
//!   decision-locked field shape.
//!
//! - `parity_rule_rejects_missing_twin`: at load time, a manifest with
//!   `dispatch.kind = "tool_call", tool_id = "estimate.does_not_exist"`
//!   fails the parity check with a structured reason; the same shape
//!   covers a `rest_proxy.path = "/api/v1/missing"`. Acceptance §2 / §3
//!   of PLUGIN-MCP.md.
//!
//! - `plugin_tools_off_switch_hides_listings`: setting the host-side
//!   `mcp.plugin_tools_enabled = false` hides plugin contributions from
//!   `list_tools` while keeping core tools live, and `tools/call`
//!   against a hidden plugin tool emits an audit row with
//!   `outcome = denied`. PLUGIN-MCP.md § Off-switch hierarchy layer 4.
//!
//! Driving the handler through its pure-fn forms (`list_tools_inner`,
//! `call_tool_inner`) keeps the test in-process; the rmcp transport is
//! exercised by `tests/stdio_handshake.rs` and re-running it here
//! would double-test plumbing the doc-locked contract does not depend
//! on.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codeless_mcp::{
    rows_for_loaded_plugin, CodelessMcpHandler, InMemoryAuditSink, McpCallOutcome,
    McpContributionTable, ServerContext,
};
use codeless_tools::plugin::{
    check_mcp_parity, LoadedPersona, LoadedPlugin, McpParityCheckInputs, McpParityError,
    PluginManifest,
};
use codeless_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use serde_json::{json, Value};

/// Minimal tool standing in for the notes plugin's `notes.append`.
/// Built here rather than depending on the on-disk plugin so the test
/// exercises the MCP-contributions seam in isolation -- a regression in
/// the contribution table cannot be papered over by the plugin's own
/// tool implementation.
struct NotesAppendStub;

#[async_trait]
impl Tool for NotesAppendStub {
    fn name(&self) -> &str {
        "notes.append"
    }
    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            json!({
                "type": "object",
                "properties": {
                    "body": { "type": "string" },
                },
                "required": ["body"],
            })
        })
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let body = args
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("body required".into()))?;
        Ok(json!({ "appended": body }))
    }
}

/// `notes` plugin manifest carrying a real `[contributes.mcp]` block.
/// The TOML is inlined so the test does not need an on-disk plugin
/// dir -- `PluginManifest::from_str` is the manifest parser's
/// established test seam (see `codeless-tools/src/plugin/manifest.rs`
/// tests) and exercising it here also covers the parser's
/// `[contributes.mcp]` validation path.
fn notes_manifest_with_mcp(dispatch_toml: &str) -> PluginManifest {
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
title           = "Append a note via MCP"
description_md  = "docs/mcp/notes_append.md"
input_schema    = "schemas/notes_append_in.json"
output_schema   = "schemas/notes_append_out.json"
tier            = "write"
dispatch        = {dispatch_toml}
"#
    );
    PluginManifest::from_str(&text, Some(PathBuf::from("/tmp/notes"))).expect("parses")
}

/// Build a `LoadedPlugin` shape carrying just the manifest -- enough
/// for `rows_for_loaded_plugin` to project MCP contributions out of.
fn loaded(manifest: PluginManifest, tool_ids: Vec<String>) -> LoadedPlugin {
    LoadedPlugin {
        manifest,
        tool_ids,
        personas: Vec::<LoadedPersona>::new(),
        migrations: Vec::new(),
    }
}

/// Construct a registry pre-populated with the notes shim. Used by
/// the dispatch and off-switch tests so the `tool_call` parity rule
/// has a real twin to point at.
fn registry_with_notes() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(NotesAppendStub));
    r
}

#[tokio::test]
async fn tool_call_dispatch_round_trip() {
    // Acceptance §1 of PLUGIN-MCP.md: a `notes` plugin's
    // `notes_append` MCP tool, declared with `dispatch = { kind =
    // "tool_call", tool_id = "notes.append" }`, lands the same row
    // in the notes back-end as a direct codeless-tool call. The
    // stub here returns `{"appended": ...}` rather than writing to
    // SQLite because the doc's "same row in notes_entries" claim is
    // structurally enforced by the dispatch path -- the same
    // `ToolRegistry::get` lookup the codeless agent uses -- not by
    // the back-end specifics.
    let manifest = notes_manifest_with_mcp(r#"{ kind = "tool_call", tool_id = "notes.append" }"#);
    let registry = registry_with_notes();

    // Parity check must clear before the table is built: the host
    // never wires a contribution it could not satisfy at boot.
    let registered_tool_ids: Vec<&str> = registry.names().collect();
    check_mcp_parity(
        &manifest,
        McpParityCheckInputs {
            registered_tool_ids: &registered_tool_ids,
            registered_rest_routes: &[],
        },
    )
    .expect("parity passes for registered twin");

    let rows = rows_for_loaded_plugin(&loaded(manifest, vec!["notes.append".into()]));
    let contributions = McpContributionTable::from_rows(rows);

    let audit = InMemoryAuditSink::shared();
    let ctx = ServerContext::new(Arc::new(registry), std::env::temp_dir())
        .with_contributions(contributions)
        .with_audit(audit.clone());
    let handler = CodelessMcpHandler::new(Arc::new(ctx));

    // List surfaces the namespaced contribution.
    let listing = handler.list_tools_inner();
    let names: Vec<&str> = listing.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"notes.notes_append"),
        "plugin contribution surfaced in tools/list: {names:?}",
    );

    // Call dispatches through `tool_call -> notes.append`.
    let result = handler
        .call_tool_inner("notes.notes_append", json!({ "body": "milk" }))
        .await
        .expect("call dispatches");
    let payload = serde_json::to_value(&result).expect("serialise CallToolResult");
    let appended = payload
        .pointer("/structuredContent/appended")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .pointer("/structured/appended")
                .and_then(Value::as_str)
        });
    assert_eq!(
        appended,
        Some("milk"),
        "dispatch reached the stub; payload was {payload}"
    );

    // Audit row carries plugin_id + dispatch (decision-locked items
    // 7 + 2 of PLUGIN-MCP.md).
    let events = audit.events();
    assert_eq!(events.len(), 1, "exactly one audit row per tools/call");
    let ev = &events[0];
    assert_eq!(ev.tool_name, "notes.notes_append");
    assert_eq!(ev.plugin_id.as_deref(), Some("notes"));
    assert_eq!(ev.dispatch_kind, Some("tool_call"));
    assert_eq!(ev.outcome, McpCallOutcome::Ok);
}

#[test]
fn parity_rule_rejects_missing_twin() {
    // Acceptance §2 + §3: the parity check fires at *load* time, not
    // at first call. Two separate failures share one structured
    // shape -- a regression that swallowed either would silently
    // break Invariant 1.
    let registered_tool_ids = ["notes.append"];
    let registered_rest_routes = ["/api/v1/notes"];

    // §2 -- `tool_call.tool_id` names a non-existent tool.
    let bad_tool_call =
        notes_manifest_with_mcp(r#"{ kind = "tool_call", tool_id = "estimate.does_not_exist" }"#);
    let err = check_mcp_parity(
        &bad_tool_call,
        McpParityCheckInputs {
            registered_tool_ids: &registered_tool_ids,
            registered_rest_routes: &registered_rest_routes,
        },
    )
    .expect_err("missing tool_call twin fails parity");
    match err {
        McpParityError::UnknownToolCallTarget { tool_id, .. } => {
            assert_eq!(tool_id, "estimate.does_not_exist");
        }
        other => panic!("expected UnknownToolCallTarget, got {other:?}"),
    }

    // §3 -- `rest_proxy.path` names a non-existent REST route.
    let bad_rest = notes_manifest_with_mcp(
        r#"{ kind = "rest_proxy", method = "POST", path = "/api/v1/missing" }"#,
    );
    let err = check_mcp_parity(
        &bad_rest,
        McpParityCheckInputs {
            registered_tool_ids: &registered_tool_ids,
            registered_rest_routes: &registered_rest_routes,
        },
    )
    .expect_err("missing rest_proxy twin fails parity");
    match err {
        McpParityError::UnknownRestProxyTarget { path, method, .. } => {
            assert_eq!(path, "/api/v1/missing");
            assert_eq!(method, "POST");
        }
        other => panic!("expected UnknownRestProxyTarget, got {other:?}"),
    }
}

#[tokio::test]
async fn plugin_tools_off_switch_hides_listings() {
    // PLUGIN-MCP.md § Off-switch hierarchy layer 4 ("plugin surface
    // (host config)"): flipping `mcp.plugin_tools_enabled = false`
    // hides every plugin contribution from `tools/list` *and*
    // rejects `tools/call` against them, while keeping core tools
    // live. Acceptance §4 of PLUGIN-MCP.md.
    let manifest = notes_manifest_with_mcp(r#"{ kind = "tool_call", tool_id = "notes.append" }"#);
    let registry = registry_with_notes();
    let rows = rows_for_loaded_plugin(&loaded(manifest, vec!["notes.append".into()]));
    let contributions = McpContributionTable::from_rows(rows).with_enabled(false);

    let audit = InMemoryAuditSink::shared();
    let ctx = ServerContext::new(Arc::new(registry), std::env::temp_dir())
        .with_contributions(contributions)
        .with_audit(audit.clone());
    let handler = CodelessMcpHandler::new(Arc::new(ctx));

    // Listing hides the plugin contribution but keeps the core
    // `notes.append` codeless tool live -- "trust core, not (yet)
    // plugins" is the operator's reason for engaging this switch.
    let listing = handler.list_tools_inner();
    let names: Vec<&str> = listing.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        !names.contains(&"notes.notes_append"),
        "plugin contribution hidden by the off-switch: {names:?}",
    );
    assert!(
        names.contains(&"notes.append"),
        "core codeless tool stays live: {names:?}",
    );

    // Calling a hidden plugin tool by name surfaces the off-switch
    // explicitly so the operator-facing client can log it; the audit
    // row records `outcome = denied` with the right plugin_id.
    let err = handler
        .call_tool_inner("notes.notes_append", json!({ "body": "x" }))
        .await
        .expect_err("hidden plugin tool rejected at tools/call");
    assert!(
        err.to_string().contains("plugin_tools_enabled"),
        "off-switch message names the config knob: {err}",
    );
    let events = audit.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, McpCallOutcome::Denied);
    assert_eq!(events[0].plugin_id.as_deref(), Some("notes"));
    assert_eq!(events[0].dispatch_kind, Some("tool_call"));
}
