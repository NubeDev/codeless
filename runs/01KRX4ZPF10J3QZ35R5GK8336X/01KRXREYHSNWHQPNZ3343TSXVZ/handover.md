## Done

- Extended `PluginManifest` with `[contributes.mcp]` block: `PluginContributes`, `PluginMcp`, `PluginMcpTool`, `PluginMcpDispatch` (`tool_call` | `rest_proxy` | `mcp_forward`), `PluginMcpResource`, `PluginMcpPrompt`, `McpTier`, `McpResourceBacking`. Strict-validated at parse time (id charset, duplicate ids, uppercase HTTP method on `rest_proxy`, resource backing field presence). `mcp_forward` parses but lands a load-time `Failed`.
- Added `codeless_tools::plugin::mcp` module: `check_mcp_parity(manifest, McpParityCheckInputs)` enforcing Invariant 1 of PLUGIN-MCP.md — `tool_call.tool_id` must be in the host `ToolRegistry`, `rest_proxy.path` must be in the registered REST routes. `mcp_forward` is rejected here with stable code `mcp-forward-not-supported` (OQ-MCP-1). Helper `mcp_listing_id(plugin_id, tool_id_local)` centralises the `<plugin>.<id>` wire-name.
- Added `codeless-mcp` modules: `contrib` (`McpContribution`, `ResolvedMcpDispatch`, `McpContributionTable` with `with_enabled` off-switch) and `audit` (`McpAuditEvent`, `McpCallOutcome`, `AuditSink` trait, `NullAuditSink`, `InMemoryAuditSink`).
- Extended `ServerContext` with `contributions: McpContributionTable` and `audit: Arc<dyn AuditSink>`. The handler exposes `list_tools_inner` + `call_tool_inner` pure-fn forms: list emits the merged core+contribution catalogue, call dispatches contributions through the resolved twin (`tool_call` reaches the registered tool; `rest_proxy` returns a structured "wire-up follow-up" error today). Every `tools/call` emits one audit row carrying `plugin_id` + `dispatch_kind`. The off-switch hides contributions from listing and rejects calls with `outcome=Denied`.
- `plugins/notes/plugin.toml` now declares `[contributes.mcp]` with a single `tool_call` -> `notes.append` tool, namespaced for MCP as `notes.notes_append` per the doc.
- New `crates/codeless-mcp/tests/plugin_mcp_e2e.rs` with the three acceptance tests green: `tool_call_dispatch_round_trip`, `parity_rule_rejects_missing_twin`, `plugin_tools_off_switch_hides_listings`.
- `cargo test -p codeless-tools -p codeless-mcp` and `cargo clippy -p codeless-tools -p codeless-mcp --all-targets -- -D warnings` clean; `cargo fmt --check -p codeless-tools -p codeless-mcp` clean. Pre-existing `plugin_substrate_e2e` suite still passes (8/8) on the updated manifest.
- Side-effect: `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` `workspace = ...` pointer was rewritten to point at this worktree (it had been pinned to a sibling worktree that no longer exists, breaking every cargo invocation). Not committed (it's outside this repo) but required for the local build.

## Next

- Stages 15–19 of the plugin-substrate-runtimes job (TBD per session doc).
- Real `rest_proxy` dispatch (today returns a structured "wire-up follow-up" error; the parity check already validates the path).
- Description/schema/prompt static-file readers (paths validate today but contents are not loaded into the MCP listing).
- Host-side glue from `codeless-server`'s plugin scanner into `McpContributionTable` (wire `rows_for_loaded_plugin` from the boot path; populate `registered_rest_routes` from the axum router).

## What you need to know

- Pre-existing `codeless-cli` build failure on this branch (`missing field 'plugins' in initializer of 'AppState'` at `crates/codeless-cli/src/serve.rs:367`) is not from this stage — reproduced before any edits via `git stash`.
- `PluginMcpDispatch::McpForward` is intentionally a unit-shaped struct variant (`McpForward {}`) so a future v0.2 can add fields without a manifest shape break.
- The audit sink design uses `Arc<dyn AuditSink>` so a production server can wire its own structured-logging subscriber later; the default `NullAuditSink` drops events.
- The off-switch (`McpContributionTable::with_enabled(false)`) preserves the rows so a future hot-reload toggle does not require a registry rebuild — `visible_rows()` filters at iteration time.
- Plugin-MCP audit shape lifted from PLUGIN-MCP.md § Audit: `plugin_id` is `Option<String>` (None for core tools), `dispatch_kind` is `Option<&'static str>` matching the manifest's wire strings.

## Open questions

- Should `mcp_forward` be a load-time `Failed` reason on the *plugin* (whole plugin doesn't load) or a per-contribution skip (other tools from the plugin still load)? Current implementation: per-contribution (`check_mcp_parity` short-circuits at the first failing tool, but that is host-policy). Doc reads consistent with per-contribution but not explicit.
- The `tier` field is parsed but not yet surfaced in MCP tool annotations (OQ-MCP-3 leaned "surface it"); landing that is a one-line `Tool::new` change once rmcp supports tool annotations in the version we use.
