# Scope — plugin-substrate

## Goal

Finish the plugin substrate per [DOCS/PLUGIN-SUBSTRATE.md](../../DOCS/PLUGIN-SUBSTRATE.md)
so any new workflow (estimating, research, support-triage) ships as a
plugin folder rather than a fork of the runner. Item 1 (tools layer)
shipped via the prior `tools-layer` job; per-job MCP wiring is now
live. This job lands items 2-8 and ships plugin #0 (`notes`)
end-to-end. The estimating plugin does not start until this job is
green.

## In scope

- PS2 CommonChat extraction: one component for assistant page,
  in-job chat, in-editor AI panel
- PS3 Server-side capability derivation: tool list derived from the
  thread row, never the client `kind` prop
- PS4 Chat state moves server-side (R4 compliance via SQLite)
- PS5 Persona / thread-kind data model: `persona_id` column,
  built-in `general` and `coding` personas, immutable per thread
- PS6 Plugin manifest + registry: `plugin.toml`, static
  `load_plugin(path)`, `<plugin_id>_*` table namespacing enforced
  at load time, `codeless plugin list/info`
- PS7 Tool-result attachments: `codeless://attachment` schema ref
  renders download/preview without per-plugin UI
- PS8 Assistant agent loop: existing `ai-runner` driven from
  Assistant conversations, action-card gated tool calls
- Plugin #0 `codeless-plugin-notes` in-tree: one tool, one
  persona, one migration, markdown attachment output
- Integration tests per substrate item + notes plugin e2e

## Out of scope

- A new agent runtime (substrate doc explicitly forbids; reuse
  `ai-runner`)
- WASI plugin host or dynamic loading (static MVP only)
- A workflow DSL or visual builder
- Per-user permissions (R5 single trust boundary still holds)
- The estimating plugin itself (separate job after this one)
- Plugin uninstall / downgrade (append-only migrations only)

## Constraints

- R1 crate dependency direction: any new plugin crate stays
  host-only; do not pull plugin code into mobile-safe crates
- R2 comments explain why, never what; no emojis, no task-status
  comments, no decorative banners
- R3 one concept per file
- R4 SQLite is the source of truth; chat state and threads live
  server-side
- R5 single bearer token / single trust boundary; do not introduce
  per-user permission scaffolding
- `allowed_tools` matching: literal id or single trailing-`*`
  dotted prefix only; reject anything else at manifest load time
- Plugin-owned tables MUST be named `<plugin_id>_<table>`;
  enforce via static check on migration SQL at load time
- `default_model_family` is a codeless alias (`fast` / `smart` /
  `reasoning`); plugins must not hardcode provider model ids
- Personas should grant `attachments.read`, not raw `fs.read`,
  unless reviewer explicitly weighs the broader blast radius
- All three local gates green before each commit: `cargo test
  --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --check`

## Open questions

1. PS6 plugin registration entry point: macro (`#[codeless_plugin]
   fn register(reg: &mut ToolRegistry)`) vs explicit linker hack vs
   inventory crate. Resolve in stage PS6.
2. PS8 action-card UX: confirm whether existing `eventFormat.ts`
   tool-call events already render as cards or if minor wire
   additions are needed. Resolve in stage PS8.
3. PS5 attachments policy enum surface (`inline-thread-scoped` vs
   other values implied by the substrate doc). Resolve in stage
   PS5.
