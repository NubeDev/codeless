# Plugin Substrate — MCP contributions

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-18

Companion to [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md). This doc
defines how a codeless plugin contributes MCP tools, resources, and
prompts to third-party MCP clients (Claude Desktop, IDE plugins,
other agents). The substrate doc already establishes that plugin
tools are visible to the codeless agent through `codeless-tools`;
this doc extends that surface outward.

If anything below contradicts [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md)
or [`../SCOPE.md`](../SCOPE.md), those win.

## One-line summary

A plugin may declare `[contributes.mcp]` in `plugin.toml`. The
codeless MCP server registers those tools, resources, and prompts
alongside its own, dispatched via one of three mechanisms —
`tool_call` (direct), `rest_proxy` (re-issued through the REST
router), or `mcp_forward` (proxied to an upstream MCP server). Every
plugin MCP tool has a REST-or-tool-call twin; descriptions are
static files in the plugin bundle. Both rules are lifted from rubix
([MCP.md § Block- and node-contributed tools](../../../rubix-workspace/rubix-agent/docs/design/MCP.md#block--and-node-contributed-tools))
and exist for the same reason: parity between the public surface
and MCP, no prompt-injection via dynamic descriptions.

## Why MCP contributions, and why now

Codeless already exposes its core tools over MCP (per
[`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md) item 1). Plugin tools
are visible to the codeless agent through the same registry. The
question this doc answers is: should they *also* be visible to
external MCP clients?

**Yes**, because:

- A user running Claude Desktop alongside codeless should see the
  estimating plugin's `estimate.takeoff_add` as a tool they can call
  from any thread, not just inside the codeless Assistant. That's
  the whole point of MCP.
- The rubix workspace has already designed the governance (parity
  rule, static descriptions, off-switch hierarchy). Codeless gets
  that design for free if we lift it.
- Adding it later means a plugin author has to think about two
  registration surfaces (codeless registry vs MCP server). Adding
  it now means the second surface is a manifest decoration on the
  first.

The cost is small: the manifest section is additive, the dispatcher
is a one-of-three discriminator, the parity check is one line at
load time.

## Manifest shape

`plugin.toml` gains a `[contributes.mcp]` block:

```toml
[contributes.mcp]
# off-switch — operators flip this without touching the plugin
enabled = true

[[contributes.mcp.tools]]
id              = "takeoff_add"
title           = "Add takeoff line to active estimate"
description_md  = "docs/mcp/takeoff_add.md"   # static file, in the plugin bundle
input_schema    = "schemas/takeoff_add_in.json"
output_schema   = "schemas/takeoff_add_out.json"
tier            = "write"                      # read | write | destructive
dispatch        = { kind = "tool_call", tool_id = "estimate.takeoff_add" }

[[contributes.mcp.tools]]
id              = "render_quote"
title           = "Render the active estimate as a quote PDF"
description_md  = "docs/mcp/render_quote.md"
input_schema    = "schemas/render_quote_in.json"
output_schema   = "schemas/render_quote_out.json"
tier            = "write"
dispatch        = { kind = "rest_proxy", method = "POST",
                    path = "/api/v1/plugins/estimating/quote" }

[[contributes.mcp.resources]]
uri_pattern     = "estimate://{estimate_id}"
backing         = "plugin_table"             # plugin_table | rest_get
table           = "estimating_estimates"     # owned per item 6's namespacing

[[contributes.mcp.prompts]]
id              = "investigate_takeoff"
template        = "prompts/investigate.md"
```

Registered tool ids in the MCP server are namespaced as
`<plugin_id>.<tool_id>` — the estimator's `takeoff_add` becomes
`estimating.takeoff_add` to a Claude Desktop client. Collisions
across plugins are impossible by construction; collisions with core
codeless tools are caught at load time (manifest parse error).

Strict-validate `dispatch.kind`: only `tool_call`, `rest_proxy`,
`mcp_forward`. Unknown → manifest parse error. The four dispatch
kinds and their REST/tool twins are:

| Dispatch kind | What it does | When to use it |
|---|---|---|
| `tool_call` | MCP handler calls the plugin's registered codeless tool via `codeless-tools::Registry::call(tool_id, args)`. The default and simplest path. | Any plugin tool that's already in the codeless registry. |
| `rest_proxy` | MCP handler re-issues the call through the codeless REST router. Declared with `{method, path}`. | Plugin tools that map naturally to REST endpoints the plugin already exposes (e.g. file uploads, multi-step orchestrations). |
| `mcp_forward` | MCP handler forwards the call to a remote MCP server over a live client session owned by the plugin, validates response against the cached upstream `output_schema`. | Federation — codeless re-exports a third-party MCP server's tools as local tools under the plugin's namespace. |

There is no `wasm_direct` or `process_direct` dispatch. A WASM or
process plugin's MCP tool dispatches via `tool_call`; the runtime
flavour is the codeless-tools registry's concern, not MCP's. This
is the load-bearing simplification — one dispatcher per MCP tool,
not per runtime flavour.

## The two invariants (lifted from rubix)

### Invariant 1 — every MCP tool has a non-MCP twin

`dispatch.kind = "tool_call"` requires `tool_id` to be a registered
codeless tool at load time. Missing → manifest parse error.

`dispatch.kind = "rest_proxy"` requires `path` to be a registered
REST route on the codeless server at load time. Missing → manifest
parse error.

`dispatch.kind = "mcp_forward"` requires a configured upstream MCP
client and a successful `tools/list` response from it including the
forwarded tool. Missing or unreachable → the plugin loads in
`Degraded` state; the forwarded tool appears in the MCP listing
with a `"reachable": false` flag. Other tools from the plugin keep
working.

**Why this matters.** The MCP server never gains a code path that
bypasses the rest of the public surface. Everything an external MCP
client can do, an internal caller (codeless agent, REST client, CLI)
could already do. That's how RBAC, audit, and observability stay
uniform; that's why rubix made it a hard invariant.

### Invariant 2 — descriptions are static files

`description_md`, `input_schema`, `output_schema`, and `prompts.template`
are paths to files inside the plugin bundle. They are read at load
time and never templated against runtime data.

**Why this matters.** Prompt-injection defence. A plugin that pulls
"recent user activity" into a tool description is, by design,
unable to. Descriptions are reviewable, signed (eventually), and
diffable; they don't move when a user types something a plugin
author didn't anticipate.

A plugin that *needs* dynamic context in the tool description has
the wrong tool design — split it into smaller tools whose names
describe the action.

## Dispatch path — what happens on a Claude Desktop tool call

```
External MCP client (Claude Desktop)
   │  tools/call { name: "estimating.takeoff_add", arguments: {...} }
   ▼
codeless MCP server  (existing, not new)
   │  parity check: known tool, dispatch kind known
   │  RBAC: single bearer token (R5), or stdio parent-process identity
   │  audit log entry minted
   ▼
Dispatch
   ├─ tool_call    → codeless-tools::Registry::call(tool_id, args)
   │                  → runtime adapter (builtin / WASM / process)
   ├─ rest_proxy   → in-process REST router → handler
   └─ mcp_forward  → upstream MCP client session → remote tool call
                      → schema-validate response against cached upstream
                        output_schema
   ▼
ToolResult (or ToolError, including upstream-unreachable)
   │
   ▼
External MCP client
```

There is no second copy of business logic for MCP. The bottom of
every dispatch path is a real handler — same one the codeless agent
or a REST client would have called. Parity is structural, not
"documented and hoped for."

## Resources

MCP resources are read-only addressable entities (`estimate://abc-
123`, `notes://2026-05-18`, …). Two backings:

- `plugin_table` — codeless serves the resource from a row in the
  plugin's namespaced SQLite table (per item 6). Read-only by
  construction; the plugin SQL never sees the resource read path.
- `rest_get` — codeless serves the resource by re-issuing a GET
  against a registered REST route.

The third rubix backing (`node`) doesn't apply — codeless has no
graph nodes.

Subscriptions (push-update when the resource changes) are out of
scope for v0.1. A plugin that wants live updates uses the existing
SSE event stream the Assistant already speaks; the MCP `resources/
subscribe` shape lands when a real client asks for it.

## Prompts

MCP prompts are pre-written prompt templates a client can request.
Codeless serves them statically from the plugin bundle —
`template` is a markdown file, never runtime-templated. Argument
substitution into the prompt is the MCP client's job, not the
codeless server's. (Same prompt-injection defence as descriptions.)

## Off-switch hierarchy

Four layers, defense in depth. Lifted from rubix
[MCP.md § The off-switch](../../../rubix-workspace/rubix-agent/docs/design/MCP.md#the-off-switch-four-layers-defense-in-depth)
with the same shape:

| Layer | Mechanism | Effect |
|---|---|---|
| **Build-time** | Cargo feature `--features mcp-server` on `codeless-server` | Compliance builds ship binaries with no MCP code at all |
| **Config** | `mcp.enabled = false` in codeless config | Server starts with the MCP feature disabled; no listener |
| **Runtime** | `codeless mcp disable` (writes config, hot-reloads) | Operator kill-switch without restart |
| **Plugin surface** | `mcp.plugin_tools_enabled = false` in codeless config | Hides every plugin-contributed MCP tool from `tools/list` while keeping codeless's core MCP tools live. Useful when an operator trusts MCP for core tools but not (yet) third-party plugins |

The fourth layer is *the* reason to design MCP contributions now
rather than later: it's the difference between "MCP is on/off
globally" and "MCP is on, but only for tools the codeless team
shipped." Without it an operator's only choices are "trust every
installed plugin's MCP surface" or "no MCP at all."

A plugin can also opt out at the manifest level
(`contributes.mcp.enabled = false`); that hides only its own tools
without affecting other plugins. Useful for plugins that want to
ship MCP tools eventually but not in their first release.

## Persona allow-list and MCP

Codeless personas (item 5) restrict which tools an Assistant
*thread* can invoke. MCP clients are *not* threads — they have no
persona, they have the bearer token's full surface.

**Decision:** plugin MCP tools are visible to MCP clients
regardless of any persona's `allowed_tools`. Persona scoping is an
Assistant-thread mechanism; external MCP clients have the same
trust as any other bearer-token caller (R5 single-tenant).

This is the right answer because:

- Personas exist to keep the agent's context budget tractable, not
  to enforce security. R5 already handles security.
- An external MCP client knows what tool it wants to call; it
  doesn't need a persona to narrow its options.
- Making MCP visibility persona-aware would require an MCP client
  to pick a persona at session start, which doesn't match how MCP
  clients work.

A plugin that genuinely wants a tool *not* exposed via MCP
declares `contributes.mcp.tools` without that tool. The codeless
registry still has it; only MCP doesn't.

## Audit

Every MCP call emits an audit event matching codeless's existing
event shape (per
[`ASSISTANT-SCOPE.md`](../ASSISTANT-SCOPE.md)) with extra fields:

```json
{
  "type": "mcp.tool_call",
  "session_id": "...",
  "plugin_id": "estimating",
  "tool_id":   "estimating.takeoff_add",
  "args_hash": "sha256:...",
  "dispatch":  "tool_call",
  "outcome":   "ok" | "err" | "denied",
  "duration_ms": 312
}
```

Plugin provenance (`plugin_id`) is a first-class field, not a
suffix on the tool id parsed at query time. This is what makes
"disable plugin X's MCP tools" a one-line config change and what
makes incident review actually possible.

## Acceptance

MCP contributions are done when:

1. The `notes` plugin (substrate item #0) registers a `notes_append`
   tool over MCP via `dispatch = { kind = "tool_call",
   tool_id = "notes.append" }`, and a Claude Desktop client (or any
   MCP client) calling `tools/call { name: "notes.notes_append",
   ... }` results in the same row in `notes_entries` as calling
   through the codeless Assistant.
2. The parity check rejects a manifest with `dispatch.kind =
   "tool_call", tool_id = "estimate.does_not_exist"` at load time
   with a clear error.
3. The parity check rejects a manifest with `dispatch.kind =
   "rest_proxy", path = "/api/v1/missing"` at load time.
4. Setting `mcp.plugin_tools_enabled = false` and reloading
   removes plugin tools from `tools/list` while keeping core
   codeless tools live; setting it back restores them.
5. An audit-log integration test
   `plugin_mcp_e2e::tool_call_emits_provenance` proves the
   `plugin_id` field is present and correct on every plugin tool
   call.

## Open questions

- **OQ-MCP-1.** Do we ship `mcp_forward` in v0.1, or defer to v0.2?
  Rubix has it for federating external MCP servers; codeless would
  use it the same way (e.g. forwarding GitHub's MCP server's tools
  under a `github.*` plugin). **Lean: defer to v0.2.** It's a real
  piece of design (upstream session lifecycle, schema caching,
  reconnect, degraded state) and adds nothing for plugin #0
  (`notes`). Land `tool_call` + `rest_proxy` first; add
  `mcp_forward` when the first federation use case appears.
- **OQ-MCP-2.** Should tool descriptions support markdown rendering
  client-side, or stay plain text? **Lean: markdown.** MCP clients
  that don't render it degrade gracefully; clients that do render
  it (Claude Desktop) benefit. Static file constraint still holds.
- **OQ-MCP-3.** Plugin manifest's `tier` (read / write /
  destructive) maps to MCP's tool annotations. Does codeless
  surface tier in `tools/list`, or just use it internally for
  approval gating? **Lean: surface it.** MCP clients can use it to
  warn users; tier is already on the tool, no extra cost.
- **OQ-MCP-4.** Resource subscriptions (push). Not in v0.1.
  Codeless's existing SSE event stream already covers the live-
  update case for the Assistant; cross that bridge when an MCP
  client asks for it.

## Decisions locked

1. **Plugin MCP tools are namespaced `<plugin_id>.<tool_id>`.**
   Collision-free by construction.
2. **Three dispatch kinds in v0.1.** `tool_call`, `rest_proxy`,
   `mcp_forward` (last is design-only in v0.1, lands in v0.2).
   Nothing else.
3. **Every plugin MCP tool has a non-MCP twin** (Invariant 1). No
   MCP-only code paths.
4. **Descriptions, schemas, and prompt templates are static files
   in the plugin bundle** (Invariant 2). No runtime templating.
5. **Plugin MCP visibility is not persona-scoped.** Personas gate
   the Assistant; MCP gates by bearer token (R5).
6. **Four-layer off-switch, including a plugin-surface layer.**
   `mcp.plugin_tools_enabled = false` hides every plugin MCP tool
   while keeping core MCP live.
7. **Plugin id is a first-class audit field.** Not parsed from the
   tool id at query time.
8. **Reuse from rubix is at the design level, not the code level.**
   The parity rule, static-description rule, dispatch discriminator,
   and off-switch hierarchy are lifted verbatim. The codeless MCP
   server implementation does not depend on any rubix crate.
