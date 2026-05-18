# Scope — plugin-substrate-runtimes

The full design lives in
**[`DOCS/plugins/PLUGIN-SUBSTRATE.md`](../../../DOCS/plugins/PLUGIN-SUBSTRATE.md)**
and its three companion docs:

- [`DOCS/plugins/PLUGIN-WASM.md`](../../../DOCS/plugins/PLUGIN-WASM.md)
  — WASM runtime flavour (item 9).
- [`DOCS/plugins/PLUGIN-UI-FEDERATION.md`](../../../DOCS/plugins/PLUGIN-UI-FEDERATION.md)
  — Module-federated UI (item 10, introduces **R6**).
- [`DOCS/plugins/PLUGIN-PROCESS.md`](../../../DOCS/plugins/PLUGIN-PROCESS.md)
  — Process runtime flavour, design-only seam (item 11).
- [`DOCS/plugins/PLUGIN-MCP.md`](../../../DOCS/plugins/PLUGIN-MCP.md)
  — MCP contributions surface.

This brief is the trimmed per-job scope. **On any disagreement, the
plugin docs win and the brief should be updated, not the docs.**

Plugin substrate items 1, 3, 5, 6, 7, 8 are already complete on
master (see [`PLUGIN-SUBSTRATE.md` § Status (stage PS-ACCEPT, 2026-05-17)](../../../DOCS/plugins/PLUGIN-SUBSTRATE.md)).
Plugin #0 `notes` ships at `plugins/notes/` and
`crates/codeless-plugin-notes/`; the runtime wiring inside
`NotesAppend::call` is a known one-tick follow-up that this job does
**not** subsume.

What this job adds: the **runtime-flavour substrate extension** —
items 9, 10, 11 (process is reserve-the-seam only), plus the
plugin-MCP contribution surface. Reuse from rubix is per-file and
port-not-depend, exactly as
[`TOOLS-PORTING.md`](../../../DOCS/TOOLS-PORTING.md) ported moxxy.

## Goal

Land items 9 (WASM), 10 (MF UI + R6), and a manifest-only reservation
for item 11 (process), plus the MCP-contribution surface, on `master`
via the `codeless/plugin-substrate-runtimes` branch. After this job:

1. A plugin author writes one `Tool` impl in Rust and ships it as
   either a builtin crate (`cargo build`) or a `.wasm` artefact
   (`cargo build --target wasm32-wasip2`) — same source, two
   packaging choices, runtime flavour chosen per-server by config.
2. The `notes` plugin builds as both flavours; the substrate
   end-to-end test
   (`plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_
   addressable_by_thread`) passes against both with the flavour
   swapped via config — no test-code change.
3. A plugin may ship a Module Federation UI bundle; the host
   (`ui/codeless-ui/`) registers the remote and mounts it at named
   slots (`assistant-panel`, `tool-result:<tool_id>`,
   `persona-picker:<persona_id>`, `settings-page:<plugin_id>`).
4. **R6 is added to `codeless/CLAUDE.md`** and enforced by an
   ESLint rule shipped from `@codeless/plugin-ui-sdk`.
5. `plugin.toml` accepts `[[runtimes]] kind = "process"` and
   strict-validates the policy block; a plugin that declares only a
   process runtime today loads in `Failed` with a structured reason
   (`"process runtime not yet supported"`). No supervisor, no proto,
   no gRPC code lands.
6. A plugin may declare `[contributes.mcp]`; tools dispatched via
   `tool_call` or `rest_proxy` are visible to third-party MCP
   clients with the rubix parity rule + static-description rule
   enforced at load. `mcp_forward` is deferred to a follow-up.

Together, these items make P2 of the substrate (one Rust crate +
data, never a fork) true for **WASM-authored** plugins as well as
builtin, and make plugin UI a real contribution surface — the two
biggest gaps in the current substrate.

## In scope

### Crates and code (Rust)

- New host-only crate **`codeless-plugin-host-wasm`** — Wasmtime
  engine + WASI-p2 component-model bindings, per-call instantiation,
  fuel/memory/wall-clock caps from `HostPolicy`. Capability sandbox
  is default-deny; manifest `[runtimes.capabilities]` grants are
  validated at load. Capability set in v0.1: `attachments` (read /
  write), nothing else; `fs`, `http`, `wall_clock` parse but produce
  `Failed` if any non-empty value appears in v0.1.
- New crate **`codeless-plugin-sdk`** (mobile-safe; `default-
  features = ["builtin"]`, optional `wasm`, optional `process`-
  reserved) with the rubix-lifted `compile_error!` mutual-exclusion
  guard. Exposes:
  - `#[derive(Tool)]` macro (lifted shape from rubix
    `#[derive(NodeKind)]`) that emits the codeless tool manifest
    and JSON schemas via `schemars`.
  - `register!` macro that expands to `inventory::submit!` under
    `builtin` and the WIT `export tool` glue (`wit-bindgen`) under
    `wasm`.
  - `ToolBehavior` trait — single authoring API; mirrors what the
    `notes` plugin already implements but with the runtime adapter
    moved into the SDK.
- New crate **`codeless-tool-wit`** — the
  `crates/codeless-tool-wit/wit/tool.wit` interface (the codeless
  tool ABI) and the generated `wit-bindgen` bindings committed
  in-tree (per OQ-WASM-2 lean).
- **`codeless-tools`** (existing): adds the runtime-adapter table
  resolving a tool id to a dispatcher (builtin call vs WASM call).
  Table is mobile-safe (`HashMap<ToolId, Adapter>`); each Adapter
  trait object lives behind a Cargo feature so host-only adapters
  (`WasmAdapter`) never compile into mobile builds. **OQ-WASM-1
  decision recorded here: table stays in `codeless-tools`, host-
  only adapters in their respective host crates.**
- **`codeless-plugin-notes`**: rewritten against
  `codeless-plugin-sdk` so the same source compiles to both
  flavours. Existing `NotesAppend::call` body kept; only the trait
  shape and registration change. The deferred SQLite-table-writer
  follow-up is **explicitly out of scope** here.
- **`codeless-server`**: manifest parser extension for `[[runtimes]]`
  (kinds `builtin`, `wasm`, `process`), `[runtimes.capabilities]`,
  `[runtimes.policy]`, and `[contributes.mcp]`. Strict validation
  rejects unknown kinds, unknown capability names, missing artefact
  paths. Two-phase scan (lifted from rubix
  `extensions-host::registry`) so a partial failure cannot leave
  the tool / persona / migration registries half-populated.
- **MCP server (existing)**: register plugin-contributed tools
  with namespacing `<plugin_id>.<tool_id>`. Two dispatch kinds in
  v0.1: `tool_call`, `rest_proxy`. Parity check at load:
  `tool_call.tool_id` must be a registered codeless tool;
  `rest_proxy.path` must be a registered REST route. Audit event
  format extended with `plugin_id`, `dispatch`. Off-switch
  `mcp.plugin_tools_enabled = false` (and per-plugin
  `contributes.mcp.enabled = false`) hide plugin MCP tools.

### UI (TypeScript / React)

- New in-tree package **`ui/codeless-ui/packages/plugin-ui-sdk/`** —
  fork of `rubix-workspace/extension-ui-sdk` at a recorded SHA.
  Renames:
  - `@rubix/agent-client` → `@codeless/rpc` (existing).
  - `@rubix/ui-core` → `@codeless/ui-core` (existing).
  Drops rubix git history; every ported file carries
  `// codeless-ported-from: rubix-workspace/extension-ui-sdk/<path>@<sha>`.
- **`@codeless/plugin-ui-sdk`** exports:
  - `mf.ts` — MF remote registration helpers, rewritten against
    codeless slot vocabulary.
  - `registration.ts` — slot-contribution wiring.
  - `PluginSlot` React component — looks up contributors, lazy-
    imports the exposed module, mounts inside an error boundary.
  - `rsbuild-shared.ts` — the single source of truth for shared-
    singleton versions (React 19, react-dom 19, zustand 5,
    `@tanstack/react-query` 5, `@codeless/rpc`,
    `@codeless/plugin-ui-sdk` itself).
  - `eslint-config.ts` — ESLint rule enforcing **R6**: a plugin
    source file must not import `@tauri-apps/*`, must not `fetch`
    the codeless server directly, must not bundle its own React /
    zustand / tanstack-query.
- **Host shell** (`ui/codeless-ui/`) gains:
  - `RpcClient.plugins.list()` call at boot enumerating enabled
    plugins with their `contributes.ui` blocks.
  - MF host registration of every enabled plugin remote against
    `/plugins/<id>/ui/mf-manifest.json`.
  - `<PluginSlot id="..."/>` mounted at each declared site.
- **`codeless-server` REST**: `GET /plugins` (reflects registry),
  `GET /plugins/<id>/ui/*` via `ServeDir` rooted at the plugin's
  `ui/` subdir.
- **`plugins/notes/ui/`** — a minimal `AssistantPanel` remote (one
  list of recent notes) wired through `@codeless/plugin-ui-sdk`,
  end-to-end-tested against the host shell.

### CLAUDE.md / hard rules

- Add **R6** to `codeless/CLAUDE.md` (Plugin MF remotes import only
  from the shared scope; no `@tauri-apps/*` outside their host
  shell, no direct `fetch` to codeless, no parallel React copy).
- Update the workspace `CLAUDE.md` "Pointers" section to list the
  four new plugin docs.

### Tests (integration coverage is the load-bearing piece)

- `plugin_wasm_e2e::wasm_plugin_respects_fuel_cap` — intentionally-
  infinite-loop plugin returns
  `tool-error { code: "limit-exceeded" }` within 200ms.
- `plugin_wasm_e2e::wasm_plugin_cannot_open_host_file` — a plugin
  with `[runtimes.capabilities] fs = []` cannot open `/etc/passwd`
  or any host file (default-deny).
- `plugin_wasm_e2e::wasm_plugin_attachment_round_trip` — a plugin
  with `attachments = ["read", "write"]` mints a new attachment,
  returns its id from `tool-result`, and that attachment renders
  inline in the Assistant thread (item 7 wiring against WASM).
- Extend `plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_
  addressable_by_thread` so the runtime flavour is parameterised
  (`builtin` and `wasm`); both pass with no test-code change.
- `plugin_ui_e2e::host_loads_plugin_remote_and_mounts_assistant_
  panel` — Playwright: with `notes` plugin enabled, the Assistant
  in the `notes` persona renders the plugin's `AssistantPanel`
  remote at the `assistant-panel` slot.
- `plugin_ui_e2e::mismatched_react_fails_loudly` — a fixture plugin
  pinning React ^18 fails MF version negotiation with a structured
  error in the slot, not a host crash.
- `plugin_ui_e2e::r6_eslint_rejects_forbidden_imports` — building
  a fixture plugin source that imports `@tauri-apps/api/core`
  fails the lint with a clear message.
- `plugin_mcp_e2e::tool_call_dispatch_round_trip` — a Claude
  Desktop-equivalent MCP client calling
  `tools/call { name: "notes.notes_append", … }` produces the same
  result and the same audit-event shape as calling the tool through
  the codeless agent.
- `plugin_mcp_e2e::parity_rule_rejects_missing_twin` — a plugin
  manifest with `dispatch.kind = "tool_call", tool_id = "does.not.
  exist"` fails to load with a clear error.
- `plugin_mcp_e2e::plugin_tools_off_switch_hides_listings` —
  setting `mcp.plugin_tools_enabled = false` and reloading removes
  plugin tools from `tools/list` while keeping core codeless MCP
  tools live.
- `plugin_substrate_e2e::process_runtime_declared_today_loads_
  failed_with_structured_reason` — a plugin declaring only
  `kind = "process"` loads in `Failed` with the documented reason
  string; no panic, no half-registered state.

### Documentation

- Update **`DOCS/plugins/PLUGIN-SUBSTRATE.md`** "Status" section
  with items 9, 10, 11 acceptance rows.
- Update **`DOCS/plugins/PLUGIN-WASM.md`** "Open questions" with
  the resolved decisions for OQ-WASM-1 / OQ-WASM-2 (record the
  outcome inline; do not leave the question open after stage 1).
- Update **`DOCS/plugins/PLUGIN-UI-FEDERATION.md`** with the final
  slot vocabulary as shipped (in case stage 1 adjusts it).
- Update **`DOCS/plugins/PLUGIN-MCP.md`** with the resolved
  decision for OQ-MCP-1 (`mcp_forward` deferred to v0.2; record
  outcome inline).
- Update **`CODELESS.md`** with one line per landed item under
  "What works today."

## Out of scope

- **Plugin #1 `estimating`.** It remains gated on substrate items
  2 + 4 (CommonChat extraction + chat state moves server-side) per
  `PLUGIN-SUBSTRATE.md` "Status" — this job does not unblock that.
- **The runtime-table writer in `NotesAppend::call`.** The plugin
  ships its `notes_entries` row + attachment-bound result as a
  follow-up tick; this job keeps the current structured `Failed`
  return path and tests against the substrate seams, not the body.
- **Process runtime implementation.** Only the manifest-acceptance
  seam lands. No `tool.proto`, no supervisor, no `process-wrap`
  integration, no gRPC. `PLUGIN-PROCESS.md` carries the design but
  this job lands none of it.
- **`mcp_forward` dispatch kind.** Manifest parser accepts the
  literal `"mcp_forward"` and fails the plugin in `Failed` with
  `"mcp_forward not yet supported"`; the real federation
  supervisor is a follow-up.
- **Hot reload of WASM plugins in dev.** `PLUGIN-WASM.md` § "Hot
  reload (dev only)" is a documented future, not a v1 deliverable.
- **WASM kv interface (`codeless:db/kv@0.1.0`).** Stateful WASM
  plugins ship as builtin until the kv WIT is designed (OQ-WASM-3).
- **Mobile shell wiring of plugin UI.** The host shell change is
  browser + desktop; mobile (Phase 6) inherits when it lands.
- **A plugin marketplace, signing, or sandboxed CSP per remote.**
  R5 single-tenant still holds; signing reservation is documented
  in PLUGIN-SUBSTRATE but lands later.
- **Drive-by refactors of `codeless-tools` beyond adding the
  adapter table.** R4 (codeless/CLAUDE.md): three similar lines is
  better than a premature abstraction.

## Constraints

- **R1 — mobile-safety is testable.** Mobile-safe crates today
  (`codeless-types`, `codeless-rpc`, `codeless-client`,
  `codeless-tools`) must continue to compile with the mobile
  feature set. Adding the runtime-adapter table to `codeless-tools`
  keeps the table mobile-safe; the host-only adapters live in
  `codeless-plugin-host-wasm` (gated by a host-only Cargo feature).
  CI verifies via `cargo check -p codeless-client --target
  aarch64-apple-ios` and `--target aarch64-linux-android` matrix
  rows (existing rows; we add the wasm host crate to the host-only
  exclusion list).
- **R2 — only `RpcClient`.** Plugin UI bundles do not import
  `@tauri-apps/*` or `fetch` codeless directly. The new
  `@codeless/plugin-ui-sdk` re-exports React, zustand, and
  tanstack-query from MF shared scope; importing them directly
  inside a plugin's source bypasses the singleton and fails the
  R6 ESLint rule (which extends R2 to plugin code).
- **R3 — one UI framework, forever.** No per-shell `.tsx`. The
  plugin author writes one React component tree; the host injects
  the RpcClient impl per shell.
- **R4 — SQLite is the source of truth.** Plugin migrations and
  contribution registrations are persisted; the host reads from
  SQLite at boot. Plugin in-memory state does not bypass this.
- **R5 — single bearer token.** No per-plugin auth scopes, no
  persona-scoped MCP visibility. Plugin tools are visible to MCP
  callers under the same bearer token; off-switches are operator-
  level only.
- **R6 — plugins import only from shared scope (NEW).** Defined
  in `PLUGIN-UI-FEDERATION.md`; this job adds it to
  `codeless/CLAUDE.md` "Hard rules" and ships the ESLint rule that
  enforces it at build time.
- **CLAUDE.md comment + file rules.** No emojis, no decorative
  banners, no task-status comments; one concept per file; no
  drive-by refactors; no half-finished implementations.
- **MSRV 1.78** for all Rust changes. `wasm32-wasip2` requires
  rustc 1.78+, which is already codeless's pinned MSRV.
- **`pnpm -C ui/codeless-ui lint`, `pnpm -C ui/codeless-ui test`,
  `cargo test --workspace`, `cargo clippy --workspace --all-
  targets -- -D warnings`, `cargo fmt --check` all green.**
- **Mani for commit + push.** Workspace-root commands only; never
  raw git from inside the worktree. No `--force`, no
  `--no-verify`. If a hook fails, fix the cause.

## Deliverables (what "done" looks like)

1. `codeless/plugin-substrate-runtimes` branch with one commit per
   stage, pushed via mani.
2. `cargo test --workspace` green; the five new
   `plugin_wasm_e2e::*` and three new `plugin_mcp_e2e::*` and
   one `plugin_substrate_e2e::process_runtime_declared_today_
   loads_failed_with_structured_reason` tests pass.
3. `pnpm -C ui/codeless-ui test` green; the two new Playwright
   tests (`host_loads_plugin_remote_and_mounts_assistant_panel`,
   `mismatched_react_fails_loudly`) and the R6-ESLint test pass.
4. `cargo clippy --workspace --all-targets -- -D warnings` green;
   `cargo fmt --check` green.
5. `pnpm -C ui/codeless-ui lint` green; **zero** new
   `@tauri-apps/*` imports outside `src/shells/desktop/` or
   inside any plugin source.
6. Manual smoke (Browser shell, dev server): the `notes` plugin
   appears in `GET /plugins`; the Assistant in the `notes`
   persona renders the plugin's `AssistantPanel` remote in the
   right-hand drawer; a Claude-Desktop-equivalent MCP client (mock
   in CI; real Claude Desktop optional locally) sees
   `notes.notes_append` in its tool listing.
7. `DOCS/plugins/PLUGIN-SUBSTRATE.md` "Status" section gains
   acceptance rows for items 9, 10, 11; OQ-WASM-1, OQ-WASM-2,
   OQ-MCP-1 resolved inline in their respective docs.
8. `codeless/CLAUDE.md` gains **R6**; the workspace `CLAUDE.md`
   "Pointers" lists the four plugin docs.

## Open questions — resolved in stage 1 (2026-05-18)

The four plugin docs ship with explicit open questions; this job
resolves the ones blocking implementation and defers the rest. Each
bias was reviewed against the relevant plugin doc and against the
WORKFLOW.md halt rule ("a resolution implying a redesign of the
plugin doc is a signal the bias was wrong"). In every case the bias
held — the doc body, the cargo-feature shape, and the test list in
this brief are already aligned with the bias. The resolutions below
are therefore the bias as stated, with the one-line *why* recorded.

1. **OQ-WASM-1 — runtime-adapter table placement.** *Resolved: table
   lives in `codeless-tools` (mobile-safe), adapters live in their
   respective host-only crates (`codeless-plugin-host-wasm`, and the
   future `codeless-plugin-host-process`), gated by Cargo features
   so they never link into mobile builds.* Why: the table is a
   `HashMap<ToolId, Box<dyn RuntimeAdapter>>` — no host-only types
   in its signature. Moving the table to a new mobile-safe
   `codeless-plugin-dispatch` crate would add a layer for no
   testable benefit; the iOS/Android cargo-check matrix in the
   constraints section already detects a host-only leak if one
   appears, and the fallback ("table moves to a new crate") remains
   on the table without any code change required today.
2. **OQ-WASM-2 — wit-bindgen output: commit or `build.rs`.**
   *Resolved: commit the generated bindings in-tree under
   `crates/codeless-tool-wit/src/bindings.rs` (or equivalent path),
   regenerated by a documented `cargo xtask` invocation, never by
   `build.rs`.* Why: a WIT change *is* an ABI change, and the
   bindings diff is the load-bearing review artefact for that ABI
   change. Hiding it behind `build.rs` makes reviewer archaeology
   worse and lets ABI drift slip into a "merge cleanly" PR. The
   bindings file carries the `// codeless-ported-from:` header if
   any rubix `wasm.rs` glue is lifted alongside.
3. **OQ-WASM-4 — hot reload on migration change.** *Resolved: refuse
   to hot-reload a plugin whose `plugin.toml` migration list has
   changed since the last load. The host emits a structured error
   with code `migration-changed-restart-required` and the operator
   restarts the server.* Why: migration replay against a running
   server is a separate piece of design (substrate item 6, OQ-PS-5);
   bolting it on under the WASM banner would couple two unrelated
   features and reach across registry boundaries. Hot reload of the
   *code* (same migrations) stays in scope as a documented future
   under `PLUGIN-WASM.md` § "Hot reload (dev only)", but not in v0.1.
4. **OQ-WASM-5 — fuel/memory/deadline knobs surface.** *Resolved:
   `HostPolicy` carries the global defaults; the codeless config
   file (`config.toml`) admits a `[plugins.<id>]` block that may
   override **downward** (`fuel`, `memory_max_bytes`,
   `deadline_ms`); the plugin manifest itself cannot set these
   fields and the manifest parser rejects them. Overrides must be
   `≤` the global default — an attempt to raise produces a config
   parse error at boot.* Why: the plugin author cannot enlarge
   their own sandbox; only the operator can, and only by editing
   the file the operator owns. Per-call knobs from inside the
   plugin manifest were considered and rejected — they invert the
   trust direction.
5. **OQ-MCP-1 — `mcp_forward` in v0.1.** *Resolved: defer to v0.2.
   v0.1 ships exactly two dispatch kinds: `tool_call` (codeless
   tool registry) and `rest_proxy` (registered REST route). The
   manifest parser accepts the literal string `"mcp_forward"` and
   the plugin loads in `Failed` with the structured reason
   `"mcp_forward not yet supported"`, exactly the same shape as
   the process-runtime seam.* Why: federating an upstream MCP
   server is a real piece of design (session lifecycle, schema
   cache, reconnect, degraded state) and the plugin-#0 `notes`
   acceptance does not exercise it. Landing `tool_call` +
   `rest_proxy` first lets the parity rule and the off-switch
   surface bake against a real test bed before the third dispatch
   kind opens.
6. **OQ-UI-1 — slot vocabulary versioning.** *Resolved: the
   `@codeless/plugin-ui-sdk` semver pin **is** the slot-vocabulary
   contract. The host reads each plugin's declared SDK version from
   its `package.json` (`@codeless/plugin-ui-sdk` in `dependencies`),
   refuses to mount a plugin whose declared SDK major.minor is
   newer than the host's, and degrades to "no slot mounted, structured
   error in the slot's error boundary" rather than crashing the
   host.* Why: a separate "slot vocabulary version" field would
   drift from the SDK version it claims to pin; the package version
   is the artefact every plugin already declares, so it carries the
   contract without inventing a parallel one.
7. **Slot vocabulary final v0.1 set — locked.** *Resolved: ship the
   five slots already documented in `PLUGIN-UI-FEDERATION.md` §
   "Slot vocabulary":*
   - `assistant-panel`
   - `tool-result:<tool_id>`
   - `persona-picker:<persona_id>`
   - `settings-page:<plugin_id>`
   - `composer-attachment-action:<plugin_id>`

   Why: every slot has a host renderer with a fallback path and at
   least one real-plugin use case (`notes` exercises
   `assistant-panel` end-to-end; the other four are exercised by
   the documented estimator design and by the host's own fallback
   tests). No slot was dropped because each removes a class of
   plugin UI we already know we want. Growing the set requires a
   host-side change per the doc, exactly as before.

No further open question from the four plugin docs blocks
implementation. The remaining OQs (`OQ-WASM-3` — kv interface;
`OQ-UI-2/3/4` — SSR, per-remote sandbox, plugin theming; `OQ-MCP-2/3/4`
— markdown descriptions, tier surfacing, resource subscriptions) stay
documented as future work in their respective plugin docs and do not
gate this job.

## References

- Plugin substrate (authoritative):
  [`DOCS/plugins/PLUGIN-SUBSTRATE.md`](../../../DOCS/plugins/PLUGIN-SUBSTRATE.md)
- Plugin docs:
  [`PLUGIN-WASM.md`](../../../DOCS/plugins/PLUGIN-WASM.md),
  [`PLUGIN-UI-FEDERATION.md`](../../../DOCS/plugins/PLUGIN-UI-FEDERATION.md),
  [`PLUGIN-PROCESS.md`](../../../DOCS/plugins/PLUGIN-PROCESS.md),
  [`PLUGIN-MCP.md`](../../../DOCS/plugins/PLUGIN-MCP.md)
- Reuse precedent: [`DOCS/TOOLS-PORTING.md`](../../../DOCS/TOOLS-PORTING.md)
- Rubix sources (read-only reference; never a Cargo dependency):
  `rubix-workspace/extension-sdk/extensions-sdk/src/lib.rs`,
  `wasm.rs`, `process.rs`;
  `rubix-workspace/extension-ui-sdk/src/`;
  `rubix-workspace/rubix-agent/crates/extensions-host/src/`;
  `rubix-workspace/rubix-agent/docs/design/extensions/EXTENSIONS.md`;
  `rubix-workspace/rubix-agent/docs/design/MCP.md`.
- Agent rules: [`CLAUDE.md`](../../../CLAUDE.md),
  [`codeless/CLAUDE.md`](../../../CLAUDE.md)
- UI architecture:
  [`DOCS/UI-ARCHITECTURE.md`](../../../DOCS/UI-ARCHITECTURE.md)
- Project scope: [`DOCS/SCOPE.md`](../../../DOCS/SCOPE.md)
