# Plugin Substrate — scope

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-17

## Summary

Codeless today is a staged AI runner whose only first-class job kind
is *coding*. The shape of the runner — agents, tools, review gates,
SQLite as truth, multi-shell UI — is domain-free. This doc proposes
the **core work** that turns that latent generality into an explicit
**plugin substrate**: a small set of primitives that let any workflow
("estimating", "research", "report-building", "support-triage") ship
as a self-contained plugin rather than a fork of the runner.

The estimating tool is used throughout as the **worked example** so
the substrate is grounded against a concrete shape, not designed in
the abstract. The substrate itself ships nothing estimating-specific.

If anything below contradicts [`SCOPE.md`](./SCOPE.md), **SCOPE.md
wins** until this proposal is merged into it. This doc partially
**supersedes** the open questions in
[`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md) (capability derivation,
agent loop, tool-result attachments) and **builds on**
[`TOOLS-PORTING.md`](./TOOLS-PORTING.md) (the `codeless-tools` crate
and the MCP registration seam).

## Why this scope (the constraints)

Two constraints determine what is in this doc and what is not.

**Constraint P1 — codeless's value is the runner, not the domain.**
The hard parts (long-running jobs that survive a closed tab, SSE event
stream, review gates, cost caps, multi-provider AI, mobile-safe crate
split, single bearer token across four shells) are already built and
already domain-free. A new domain (estimating, research, anything)
must reuse those parts verbatim or it is the wrong shape. This rules
out forking the runner per domain and rules out a parallel chat
surface per domain.

**Constraint P2 — a plugin is data + a small Rust crate, never a
fork.** Domain experts (the HVAC estimator, the research analyst)
edit CSV, prompts, and templates. Engineers add a thin Rust crate
that registers tools. If shipping a new workflow requires touching
`codeless-runtime`, `codeless-rpc`, `codeless-tools`, or the UI,
the substrate has failed.

Everything below is the smallest set of additions that makes P2 true.

## Worked example — the estimating workflow

Used throughout as the concrete shape. The substrate is designed
against it, but ships nothing estimating-specific.

A user opens the Assistant
([`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md)), picks the
**Estimating** persona, drags a floor-plan PDF and a scope PDF into
the composer, and says:

> Quote a mini-split for this 3-bed bungalow, Sydney.

The agent (the existing `ai-runner`, no fork) calls a sequence of
plugin-registered tools — `estimate.new`, `estimate.extract_scope`,
`catalog.search`, `estimate.takeoff_add`, `estimate.bom_compute`,
`estimate.render_quote` — each mutation gated by an action card the
user confirms in chat. The final tool returns an attachment id; the
rendered PDF appears inline in the thread.

Nothing in that flow is estimating-shaped at the substrate layer.
Substitute "research → cite → synthesize → render report" or
"support ticket → triage → draft reply → file in CRM" and the
machinery is identical. Only the plugin's tools, persona, and data
change.

## What "core work" means here

Eight items, in dependency order. Items 1–4 are **finish what's
already started** (extraction, security cleanup, R4 compliance).
Items 5–8 are **new substrate** and are the meat of this doc.

The estimating tool comes after this substrate; it does not start
until items 1–6 are landed and 7–8 have a concrete shape. The
worked example below shows what each item gives it.

### 1. Tools layer is real and registrable

Status: proposed in [`TOOLS-PORTING.md`](./TOOLS-PORTING.md), not yet
built.

The `codeless-tools` crate exists as code, with the MCP registration
seam wired up and at least two ported primitives proving the shape.
Without this, "plugin" has nothing to plug into.

Acceptance: a plugin crate can call a single `register_tool(...)` API
exposed by `codeless-tools` and have its tool appear to the agent over
MCP and to any third-party MCP client, with no further glue. (A CLI
discovery surface — `codeless plugin list` / `codeless tools list` —
is scoped under item 6's manifest reader, not this item.)

### 2. CommonChat extraction is done

Status: scoped in [`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md)
Surfaces §2, not yet executed.

`JobChat` (in `RunPane.tsx`), `AiChat`, and the chat store collapse
into one `CommonChat` component with one server-side conversation
model. The plugin UX assumes one chat surface; ship the extraction
first or every plugin will reinvent attachments and streaming.

Acceptance: the assistant page, the in-job chat, and the in-editor AI
panel all render the same `CommonChat` component bound to a
server-resident thread id.

### 3. Server-side capability derivation

Status: implied by [`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md)
Cross-cutting rules (R4 + R5), not yet enforced.

The set of tools a thread can invoke is derived **server-side** from
the thread row, never trusted from the client's `kind` prop. A
malicious or buggy client passing the wrong `kind` must not be able to
invoke tools the runner wouldn't otherwise allow on that thread.

This rule is the prerequisite for the persona model in item 5 — a
persona is meaningless if the client can override its allowed-tools
list.

Matching rule for a persona's `allowed_tools` (item 5, item 6): each
entry is either a literal tool id (`fs.read`) or a dotted-prefix glob
ending in `.*` (`estimate.*`). No shell globbing, no regex. A tool id
matches an entry iff (a) it equals the literal, or (b) the entry ends
in `.*` and the tool id starts with the entry's prefix plus a dot.
This is the only matching semantics the runner implements; item 6's
manifest reader rejects any other syntax at load time.

Acceptance: removing the `kind` prop from a `CommonChat` call site
does not change which tools the runner will execute on that thread.

### 4. Chat state moves server-side (R4 compliance)

Status: noted in [`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md)
Surfaces §3, not yet migrated.

`modules/ai/store/chatStore.ts` still holds conversation state
client-side. R4 ([`SCOPE.md`](./SCOPE.md#r4-sqlite-is-the-source-of-truth))
requires SQLite be the truth; plugins assume threads are durable and
replayable via `RpcClient.subscribe()`.

Acceptance: closing the browser, restarting the server, and reopening
the same thread reproduces the full conversation including in-flight
tool calls.

### 5. Persona / thread-kind data model

Status: **new**.

A `persona` is a server-resident bundle of `(system_prompt,
allowed_tools[], default_model, default_attachments_policy)`. A thread
declares its persona at creation and the persona is immutable for that
thread's life.

Built-in personas:

- `general` — the existing Assistant default.
- `coding` — the existing job-runner persona, reified.

Plugin-supplied personas ship in the plugin's manifest (item 6) and
register at startup.

Why this is load-bearing:

- Without personas, every tool every plugin ever registers is visible
  to every thread. Context budget blows up; the agent gets confused;
  the security story in item 3 has no labels to enforce against.
- With personas, the estimator's HVAC thread sees only estimating
  tools; the coding thread sees only coding tools; nothing crosses.

Acceptance: `assistant_threads` has a `persona_id` column; the runner
loads tools and system prompt from that persona at agent-call time;
omitting `persona_id` is a hard error, not a silent default.

### 6. Plugin manifest and registry

Status: **new**. The single largest unbuilt piece.

A plugin is a directory containing a `plugin.toml`:

```toml
[plugin]
id        = "estimating"
version   = "0.1.0"
crate     = "codeless-plugin-estimating"

[[personas]]
id                          = "estimating"
prompt_file                 = "prompts/system.md"     # relative to the plugin dir
allowed_tools               = [
  "estimate.*",
  "catalog.*",
  "attachments.read",       # see note below; do NOT grant raw fs.read
]
default_model_family        = "smart"                 # codeless alias, not a provider model id
default_attachments_policy  = "inline-thread-scoped"

[migrations]
dir = "migrations"

[data]
# domain-specific assets the plugin loads at startup
dir = "domains"
```

The canonical list of tools a plugin registers comes from the
`register_tool(...)` calls its crate makes at startup (item 1). The
manifest deliberately does not enumerate tools; if it did, the two
sources would skew. `codeless plugin info` reads the registry, not
the manifest, for tool listings.

`default_model_family` is a codeless-side alias (`"fast"`, `"smart"`,
`"reasoning"`) that the runner resolves to a concrete provider/model
at call time. Plugins must not hardcode provider model ids — the
mapping lives in codeless config and changes when models do.

`prompt_file` and `migrations.dir` / `data.dir` are paths relative to
the plugin directory.

**Attachment access, not raw FS.** Personas should grant
`attachments.read` (which scopes reads to the persona's own thread's
attachments) instead of `fs.read` (host filesystem). R5 is single-
tenant, but the blast radius of "the LLM can read anywhere the server
process can" is still larger than "the LLM can read this thread's
uploads." Plugins that genuinely need broader host FS access must
say so explicitly and the reviewer must weigh it; do not slip it in
through the persona example.

The `codeless-tools` registry (item 1) gains a `load_plugin(path)`
entry point that:

1. Parses the manifest.
2. Discovers the plugin's already-linked registration entry point
   (statically linked at build time for MVP; WASI plugins are a later
   question — see [TOOLS-PORTING.md] for the path) and calls it so
   the plugin's `register_tool(...)` calls fire.
3. Runs the plugin's SQLite migrations against the codeless DB.
   Migrations are additive only and namespaced — every plugin table
   is named `<plugin_id>_<table>` (see OQ-PS-4). Enforcement: the
   migration runner parses each `CREATE TABLE` / `ALTER TABLE` /
   `DROP TABLE` and rejects any statement whose target table name
   does not start with `<plugin_id>_`. This is a static check on
   migration SQL at load time, not a runtime SQLite-level constraint
   (SQLite has none to offer).
4. Registers personas from the manifest.

Why a manifest, not just code:

- The CLI can `codeless plugin list` / `codeless plugin info` without
  invoking any plugin code.
- The Assistant UI can render a "pick a persona" picker driven by
  manifest data.
- A future capability-policy layer has one place to read from.

Acceptance: a plugin crate compiled with the codeless server is
discoverable by id, its tools appear in MCP, its personas appear in
the Assistant's persona picker, and its migrations are applied
idempotently at server startup.

### 7. Tool-result attachments

Status: **new** (wire-up only; the attachment table already exists).

Attachments today flow user → server (uploads). The reverse path —
a tool returning an `attachment_id` (e.g. the rendered quote PDF)
that renders inline in the thread as a downloadable card — does not
exist.

Every artifact-producing plugin needs this. Estimator returns PDF
quotes; a report plugin returns DOCX; a research plugin returns a
citations file. Build it once.

A tool returns an attachment as a JSON object of shape:

```json
{ "attachment_id": "att_…", "mime": "application/pdf", "filename": "quote.pdf" }
```

`attachment_id` is required and authoritative; `mime` and `filename`
are advisory hints for the renderer (the server already knows them
from the attachment row and will use the stored values if the tool
omits or disagrees). A tool that returns multiple attachments returns
an array of these objects, or wraps them in a field declared as
`{ "type": "array", "items": { "$ref": "codeless://attachment" } }`
in the tool's output schema.

Acceptance: a tool whose JSON-schema output declares
`{ "$ref": "codeless://attachment" }` causes the Assistant to render
an attachment card with download link and (for images/PDF) inline
preview, without any per-plugin UI code.

### 8. Assistant agent loop

Status: **new**.
[`ASSISTANT-SCOPE.md`](./ASSISTANT-SCOPE.md) deliberately scopes this
out ("a new agent runtime is a non-goal"). That is still correct —
**no new runtime**. But something has to drive tool calls from the
Assistant conversation, and today only the job runner does that, only
for coding jobs.

The shape:

- The Assistant calls into the **existing `ai-runner`** with the
  persona's system prompt, the conversation history from SQLite, and
  the persona's allowed tool list.
- Tool-call events stream back through the existing event format
  ([`eventFormat.ts`](../ui/codeless-ui/src/modules/jobs/eventFormat.ts))
  and render as action cards.
- The user confirms; the server executes the tool via the
  `codeless-tools` registry; the result (including attachments — item 7)
  appends to the thread.

No planner, no stuck detection, no recovery — that all belongs to the
job loop. The Assistant loop is one turn of `ai-runner`, followed by
zero-or-more confirmed tool calls, followed by the next user message.

"One turn" here means: the runner is allowed to emit a single
response which may contain N tool-call requests; each requested tool
call is gated by an action card and either confirmed (executed,
result appended to the thread) or rejected (rejection appended). When
all gated calls have resolved, control returns to the user — the
runner does not auto-iterate without a new user message. Iteration
caps, per-thread cost caps, and provider error handling inherit from
the existing `ai-runner` settings in [`SCOPE.md`](./SCOPE.md); the
Assistant loop adds none of its own.

Acceptance: an Assistant thread with the `general` persona can call
one read-only tool (e.g. `list_jobs`) end-to-end, with the result
rendered as a card and persisted to SQLite. With this in place, every
subsequent plugin is "register tools and a persona."

## Worked example, revisited — what each item gives the estimator

| Substrate item | What it gives the estimating plugin |
|---|---|
| 1. Tools layer | A place to register `estimate.*` and `catalog.*` tools. |
| 2. CommonChat | The Assistant surface the user actually talks to. No estimator-specific chat UI. |
| 3. Capability derivation | The HVAC thread cannot accidentally `stop_job` on the user's coding work. |
| 4. Server-side state | A half-built quote survives a browser close. Auditable trail. |
| 5. Personas | The "Estimating" persona ships with the plugin; the user picks it once. |
| 6. Plugin manifest | The plugin is one folder. Estimating-domain folders (`hvac/`, `plumbing/`) live under it as data, not Rust. |
| 7. Tool-result attachments | `estimate.render_quote` returns a PDF that appears inline in chat. |
| 8. Assistant agent loop | The thing that actually drives the tools from the conversation. |

The estimating plugin itself is then small:

- One crate: `codeless-plugin-estimating`.
- One manifest: `plugin.toml`.
- One persona: `estimating`.
- Domain folders under `domains/` (HVAC, plumbing, ...) — CSV catalogs,
  rule files, prompt fragments, quote templates. Adding a new trade
  is a folder, not a code change.

Estimator-specific Rust code touches no crate outside its own.

## Plugin #0: `notes`

The estimator is too big to also serve as the substrate's first test
plugin. `notes` is plugin #0: one tool (`notes.append`), one persona
(`notes`), one migration (`notes_entries` table), one markdown
attachment as the rendered output. It exercises items 1–8 end-to-end
with zero domain complexity and lives in-tree alongside the substrate
so a substrate change that breaks plugin contract fails CI
immediately. The estimating plugin lands only after `notes` is
green.

### 9. WASM plugin host (second runtime flavour)

Status: **new**. Full design in [`PLUGIN-WASM.md`](./PLUGIN-WASM.md).

A plugin's tool implementations may ship as one or more `.wasm` files
loaded by a Wasmtime-based host living in a new host-only crate
`codeless-plugin-host-wasm`. The same `plugin.toml` from item 6 gains
a `[[runtimes]]` array; a plugin can declare `kind = "builtin"`
(today's static link), `kind = "wasm"` (new), or both. The
authoring crate writes a `ToolBehavior` trait impl — one trait, one
authoring API; the build target (`cargo build` vs `cargo build
--target wasm32-wasip2`) decides the packaging.

Load-bearing reasons WASM is the second flavour (not process):

- **R1 stays intact.** Wasmtime is a library; loading a `.wasm` does
  not spawn a process. The host crate is host-only but the *plugin
  artifact* is portable. A future mobile shell that wants in-app
  plugins can run trusted WASM without ever depending on
  `codeless-adapters-host`.
- **No build toolchain on the user's machine.** A `.wasm` blob drops
  into `plugins/<id>/wasm/tool.wasm` and registers. Static linking
  alone forces every plugin through a rebuild of `codeless-server`,
  killing third-party authoring.
- **Capability sandbox is native to WASI-p2.** Persona
  `allowed_tools` (item 3) controls which tools the agent may call.
  WASI capabilities control what the plugin code itself can touch
  (no host FS by default; no clock; no network unless granted). The
  two layers compose without either reaching into the other.

Acceptance: the `notes` plugin from item #0 can be rebuilt with
`cargo build --target wasm32-wasip2`, dropped into `plugins/notes/
wasm/notes.wasm`, registered through the same `PluginRegistry::
load_plugin` entry point, and pass the same end-to-end test
(`plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_
addressable_by_thread`) with the runtime flavour swapped.

### 10. Module-federated plugin UI (R6)

Status: **new**. Full design in
[`PLUGIN-UI-FEDERATION.md`](./PLUGIN-UI-FEDERATION.md).

A plugin may contribute UI in addition to (or instead of) tools, via
a Module Federation remote bundle. The host page registers the remote
at boot and mounts its exposed modules at declared slots
(`assistant-panel`, `persona-picker`, `tool-result:<tool_id>`,
`settings-page:<plugin_id>`). React, zustand, and tanstack-query are
shared singletons pinned in both host and remote `rsbuild.config.ts`.

The MF host shell lives in `ui/codeless-ui/`; the contract is
documented and tested at the codeless side and frozen on a major
version. Plugin UI is authored against an in-tree SDK package
`@codeless/plugin-ui-sdk` (a fork of rubix's `extension-ui-sdk` —
see the reuse table at the end of this doc).

**New R-rule — R6:** plugin MF remotes import `RpcClient` from the
host's shared scope and never import `@tauri-apps/*`, `fetch(...)`
to the codeless server directly, or any non-shared React. Same
reasoning as R2, applied to plugin code.

Acceptance: the `notes` plugin gains a `ui/` directory, declares
`contributes.ui.entry` in its manifest, and renders an inline
"recent notes" panel in the Assistant via MF without any host-side
code change beyond the generic MF loader.

### 11. Process plugin host (deferred, seam reserved)

Status: **deferred**. Design lifted from rubix in
[`PLUGIN-PROCESS.md`](./PLUGIN-PROCESS.md); landing waits until either
(a) a polyglot authoring story is needed (Python/Go/TS plugins), or
(b) a plugin needs crash isolation the WASM sandbox can't provide
(long-running drivers, native deps).

Reserving the seam now means `plugin.toml` accepts
`[[runtimes]] kind = "process"` and the manifest parser strict-
validates it; the actual gRPC supervisor lands later. This is the
same shape rubix uses (`block.proto` + `BehaviorProxy` +
`process-wrap` + circuit breaker), narrowed to the codeless tool
trait.

Mobile-safety note: the process host crate
`codeless-plugin-host-process` is host-only and gated behind a Cargo
feature that the mobile build does not enable, exactly like
`codeless-adapters-host`.

## Reuse from rubix — what we lift, what we don't

The rubix workspace (`rubix-workspace/extension-sdk/`,
`rubix-workspace/extension-ui-sdk/`,
`rubix-workspace/rubix-agent/crates/extensions-host/`) ships a
production three-flavour plugin system (native/WASM/process) with a
matching Module-Federation UI SDK. Codeless reuses it the same way
[`TOOLS-PORTING.md`](../TOOLS-PORTING.md) reuses moxxy — **port files,
do not depend on the upstream crate**. The author-facing model in
rubix is dataflow nodes (kinds, slots, messages, ports); codeless's
is AI tools (input schema, output schema, `call(args) → result`).
Forcing one into the other would import the entire rubix graph SPI.
The substrate plumbing, in contrast, is genuinely shared shape.

| Rubix asset | Codeless action | Effort saved |
|---|---|---|
| `extension-ui-sdk/` (TypeScript MF helpers, ~6 files + tests) | **Fork in-tree** as `ui/codeless-ui/packages/plugin-ui-sdk/`. Rename `@rubix/agent-client` → codeless `RpcClient`, `@rubix/ui-core` → codeless shadcn primitives. Drop rubix git history; we own it outright. | ~1 week of MF plumbing |
| `extensions-sdk/src/lib.rs` mutually-exclusive feature-flag pattern | Copy the `compile_error!` guard verbatim into `codeless-plugin-sdk/src/lib.rs`. Swap the flavour names if needed, keep the shape. | Half a day; gets the "one SDK, three flavours" enforcement right at compile time |
| `extensions-sdk/src/wasm.rs` (187 LoC, WASI host-function ABI) | Copy as the starting point for `codeless-plugin-sdk/src/wasm.rs`. Rewrite the envelope from `OnInputEnvelope { node_id, port, msg_json }` to `ToolCallEnvelope { tool_id, args_json }`. ABI symbols (`alloc`, `describe`, exported tool entry point) keep the same pack/leak/read_slice scaffolding. | Days; the raw-pointer/`#[allow(unsafe_code)]` dance is correct in rubix and worth not re-deriving |
| `extensions-sdk-macros` (`#[derive(NodeKind)]`) | Lift the derive-macro pattern into `#[derive(Tool)]` that generates input/output JSON schema (via `schemars`) from the struct and impls a `ToolBehavior::manifest()` returning the codeless tool manifest shape. | A day; we get tool-schema-from-struct for free |
| `extensions-host/src/registry.rs` two-phase scan + namespace check | **Pattern lift, no code lift.** Codeless's `PluginRegistry::load_plugin` (item 6) already validates manifests; extend it with the rubix "validate all, then commit" structure so a partial load can't leave the tool/persona/migration registries half-populated. Document the two-phase rule in this doc, not the rubix source. | A day of design transposition |
| `extensions-sdk/src/process.rs` (757 LoC, gRPC + UDS + supervisor) | **Reference only**, for [`PLUGIN-PROCESS.md`](./PLUGIN-PROCESS.md) when it lands. Codeless's process host will use the same shape (UDS, identity check on `Describe`, circuit-breaker supervisor) but its own proto matching the tool trait. | Weeks, if/when process lands |
| `block-client` (rubix `BootCtx`, kind registry) | **Do not lift.** Welded to rubix's graph SPI; codeless has its own runtime, RPC layer, persona model. | n/a — would be net-negative |
| `extensions-sdk/src/node.rs`, `ctx.rs`, `subscribe.rs`, `settings.rs` | **Do not lift.** All graph-node/slot/message vocabulary; no codeless analogue. The codeless `ToolBehavior` trait is `fn call(ctx: &ToolCtx, args: JsonValue) -> Result<JsonValue, ToolError>` — flat. | n/a |
| `rubix-extensions-sdk` as a Cargo dependency | **Never.** Pulls in `spi`, `block-client`, the whole rubix graph SPI. Same anti-pattern [`TOOLS-PORTING.md`](../TOOLS-PORTING.md) rejected for moxxy. | Avoids permanent upstream coupling |

Operational note: every ported file gets a `// codeless-ported-from:
<rubix-path>@<sha>` header recording origin and the SHA at port
time. There is no patch log — once ported, the file is codeless code
by every standard ([CLAUDE.md § File-level rules](../../CLAUDE.md)).
If a rubix fix is worth pulling later, it's a re-port decision, not
a merge.

## Non-goals (for this doc)

- The estimator itself. Scoped separately once items 1–6 are landed.
- The full WASM host design — that lives in
  [`PLUGIN-WASM.md`](./PLUGIN-WASM.md).
- The full MF UI contract — that lives in
  [`PLUGIN-UI-FEDERATION.md`](./PLUGIN-UI-FEDERATION.md).
- The process-flavour design — reserved seam only;
  [`PLUGIN-PROCESS.md`](./PLUGIN-PROCESS.md) carries the design but
  is explicitly not in MVP scope.
- The MCP-contribution surface (plugin tools / resources / prompts
  visible to third-party MCP clients) — that lives in
  [`PLUGIN-MCP.md`](./PLUGIN-MCP.md). The substrate's item 1
  (`codeless-tools` registry) is the prerequisite; PLUGIN-MCP is
  the outbound view of the same tools.
- A workflow DSL or visual builder. A plugin is Rust (or WASM) + data.
  If a domain needs a DSL, that DSL is a plugin-internal concern, not
  a substrate concern.
- Rhai-style scripted plugins. Considered and skipped: WASM with a
  small Rust authoring surface covers the "no manual ABI" goal, and
  codeless plugins are non-trivial (tool calls with structured I/O
  and attachments) — a Rhai tier would not pay for itself before
  WASM ships.
- Per-user permissions. R5 ([`SCOPE.md`](./SCOPE.md#r5-single-tenant-trust-boundary))
  still holds — single bearer token, single trust boundary.

## Open questions

- **OQ-PS-1.** Does a plugin own its own SQLite tables, or write into
  a generic key-value namespace owned by the substrate? Owning tables
  is simpler for the plugin author but couples the codeless DB schema
  to every installed plugin. **Decision: own-tables.** Plugins may
  not alter codeless-owned tables. Enforcement is the static check
  on migration SQL described in item 6 (statements must target
  `<plugin_id>_*` table names); no runtime SQLite-level constraint
  exists, so the check is what we have.
- **OQ-PS-2.** Are plugins compiled into the codeless binary, or
  loaded dynamically? Static for MVP is the only credible answer
  given the Rust ecosystem; dynamic loading via `libloading` or WASI
  is a Phase-7 question.
- **OQ-PS-3.** Where does a persona's system prompt come from when
  the plugin wants to compose it from per-domain fragments (HVAC
  prompt vs plumbing prompt under one `estimating` persona)? Options:
  (a) one persona per domain (`estimating-hvac`, `estimating-plumbing`),
  (b) a persona-time "domain" selector the user picks alongside the
  persona, (c) the plugin's tools themselves switch domain mid-thread.
  Lean: (a) — simplest, no new substrate concept.
- **OQ-PS-4.** Migration ordering across plugins. **Decision:
  namespace by plugin id.** Every plugin-owned table is named
  `<plugin_id>_<table>`; the manifest reader rejects migrations that
  violate the rule (see item 6). Refuse-on-collision was the
  alternative but is the wrong default — a third plugin would break
  a working install on a name clash.
- **OQ-PS-5.** Plugin upgrade and uninstall. For MVP: migrations are
  append-only (each plugin owns a monotonic migration sequence under
  its own `<plugin_id>_*` namespace); plugin uninstall is out of
  scope (data persists, the plugin is simply unlinked from the
  build). Forward-only schema evolution covers v0.1 → v0.2 changes.
  Revisit when a plugin needs destructive uninstall or a downgrade.

## Acceptance for "substrate complete"

The substrate is done when all of the following are true:

1. A new plugin can be added in one PR that touches **only** its own
   crate directory, its `plugin.toml`, and its `domains/` data. No
   change to `codeless-runtime`, `codeless-rpc`, `codeless-tools`
   (other than auto-registration), or the UI.
2. The `notes` plugin #0 ships and exercises items 1–8 end-to-end
   from the Assistant.
3. Each of items 1–8 has integration-test coverage in the codeless
   workspace; `notes` has end-to-end coverage that drives the
   Assistant → persona → tool → attachment path through MockRpcClient
   or its server-side equivalent. Without this, R4/R5 compliance
   regresses silently the next time the substrate is touched.

Until all three are true, the estimating plugin does not start.

### Status (stage PS-ACCEPT, 2026-05-17)

Items 1, 3, 5, 6, 7, 8 are **complete**, with the load-bearing
integration coverage living in:

| Item | Integration test |
|---|---|
| 1. Tools layer | `crates/codeless-tools` unit tests + `crates/codeless-plugin-notes/tests/plugin_smoke.rs` (on-disk plugin round-trips through `PluginRegistry::load_plugin`). |
| 3. Capability derivation | `crates/codeless-runtime/src/rpc/chat_capability.rs` unit tests + `plugin_substrate_e2e::persona_allowed_tools_admit_plugin_namespace_only` (PS3 acceptance: removing the `kind` prop cannot change which tools the runner executes). |
| 5. Persona / thread-kind | `crates/codeless-runtime/tests/personas_rpc.rs` + `tests/migrations.rs` (0017 + 0018 columns) + `plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_addressable_by_thread`. |
| 6. Plugin manifest + registry | `crates/codeless-tools` unit tests (manifest / migration / model-family / registry) + `crates/codeless-cli/tests/plugin_cli.rs` + `plugin_substrate_e2e::notes_plugin_directory_shape_matches_substrate_contract`. |
| 7. Tool-result attachments | `crates/codeless-runtime/src/rpc/attachment.rs` unit tests + `plugin_substrate_e2e::plugin_tool_output_schema_round_trips_through_attachment_reconciler` (stored row wins over tool hints, end-to-end through an upload). |
| 8. Assistant agent loop | `crates/codeless-runtime/src/rpc/assistant_planner.rs` unit tests (persona-bound prompt, persona-filtered catalogue, cap on emit-time tool calls) + `plugin_substrate_e2e::planner_allow_filter_admits_plugin_tool_under_persona_namespace` (load-time grant survives through SQLite to agent-call time). |

Items 2 and 4 are **partially landed**; the remaining work is
documented in
[`sessions/2026-05-17-plugin-substrate.md`](./sessions/2026-05-17-plugin-substrate.md):

- **Item 2 (CommonChat extraction).** PS2a shipped — every call site
  passes a `threadId` matching its server-resident id, pinned by
  `ui/codeless-ui/src/modules/chat/CommonChat.test.tsx`. PS2b–PS2d
  (the actual three-way collapse of `JobChat` / `AiChat` /
  `AssistantThreadView`) is halted on a server-side message model
  that does not exist yet, which is item 4 (PS4).
- **Item 4 (chat state moves server-side).** Halted. The in-editor
  AI panel's agent loop still runs in the browser
  (`@ai-sdk/react`). The shape of the migration (server-side
  `chat_threads` + agent-runtime adapter + tool ports + SSE event
  format extension + UI rewrite + data migration) is recorded in the
  session doc. Acceptance §3's "MockRpcClient or its server-side
  equivalent" half is satisfied via `InProcessRpc` in
  `plugin_substrate_e2e.rs` for the persona / tool / attachment
  path; the browser-resident chat state is the open piece.

Plugin #0 (`notes`) ships at `plugins/notes/` and
`crates/codeless-plugin-notes/`; the runtime wiring that mints
`notes_entries` rows + attachment-bound results from inside the tool
body is deferred (`NotesAppend::call` returns a structured `Failed`
naming this stage as the gap, see
`crates/codeless-plugin-notes/src/lib.rs`). The substrate seams the
tool sits on — persona-grant, registry dispatch, attachment marker —
are exercised end-to-end by `plugin_substrate_e2e.rs`; landing the
table writer + attachment mint is a one-tick follow-up that does not
touch the substrate contract.

The estimating plugin is unblocked on items 1, 3, 5–8; it remains
gated on items 2 + 4 because the worked-example flow ("user drags
PDFs into the composer, talks to the Estimating persona, gets a
quote PDF inline") presupposes the in-editor panel and the assistant
surface share one server-resident thread model. Estimator work can
begin against the Assistant page alone (which is already R4-
compliant) once a job author signs off on shipping it without
in-job-chat parity.
