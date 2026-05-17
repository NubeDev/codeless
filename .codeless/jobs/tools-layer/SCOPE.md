# Scope — tools-layer

The deep design lives in [`DOCS/TOOLS-PORTING.md`](../../../DOCS/TOOLS-PORTING.md)
(Phase 1) and the framing in [`DOCS/PLUGIN-SUBSTRATE.md`](../../../DOCS/PLUGIN-SUBSTRATE.md)
item 1. This file is the per-job brief — keep it short, follow the
links for the full context.

## Goal

A `codeless-tools` crate exists, exposes a `Tool` trait and a
registration mechanism that `codeless-mcp` consumes at startup, and
serves one real ported tool (`browse.fetch`) end-to-end through MCP.
With this in place, "plugin" has something to plug into; without it,
every later item in `PLUGIN-SUBSTRATE.md` is blocked.

## In scope

- New host-only workspace crate `codeless/crates/codeless-tools/`.
- `Tool` trait, `ToolCtx`, `ToolError`, registration mechanism, and a
  `testing::fake_ctx()` harness (TOOLS-PORTING §"Tool surface" +
  §"ToolCtx").
- `codeless-mcp` consumes a `ToolRegistry` at startup, advertises
  registered tools, dispatches MCP calls into `tool.call(&ctx, args)`.
- Codeless-side `NetworkMode` + `AllowlistFile` policy types under a
  `policy` module.
- `codeless/NOTICE` carrying moxxy's MIT notice for any ported code.
- `SCOPE.md` crate table updated to add `codeless-tools` with the R1
  host-only enforcement note.
- One ported tool: `browse.fetch` from `moxxy-runtime/src/primitives/browse.rs`
  (the `fetch` primitive only — not the HTML extractor, not the JS
  browser), with provenance comment and inline JSON schema.
- Integration test for `browse.fetch` using the T2 harness.

## Out of scope

- Any tool other than `browse.fetch`. Phase 2 picks the second tool
  *after* Phase 1 lands, based on what hurt.
- `codeless-tools::testing::fake_ctx()` growing a real worktree /
  in-memory SQLite / fake LLM — Phase 1 keeps it minimal.
- Migrating `ai-runner`'s in-tree primitives to call `codeless-tools`
  directly. TOOLS-PORTING §"ai-runner overlap" explicitly defers this.
- Plugin manifest, persona model, attachments, or any other
  PLUGIN-SUBSTRATE.md item 2-8. This job is item 1 only.
- WASI / dynamic plugin loading. Static linking is the only credible
  Phase 1 answer.
- Streaming tool results — the cheap `call` path is enough for Phase 1.

## Constraints

- R1 (CLAUDE.md): `codeless-tools` is host-only. `cargo check`-level
  enforcement via the `host` Cargo feature; grep for `process::Command`
  outside `codeless-adapters-host` and `codeless-tools` must return
  zero matches.
- R2 (file-level rules): one concept per file. Trait + its types in
  one file, registration in another, policy types in their own module.
- R5 (tests live with the code): every public type lands with a test;
  the harness lives in `codeless-tools::testing` so downstream callers
  can use it without re-inventing fakes.
- MSRV 1.78. `cargo clippy --workspace --all-targets -- -D warnings`
  green. `cargo fmt --check` green.
- Ported code from moxxy preserves MIT via `codeless/NOTICE` and a
  per-file provenance comment.
- The `Tool` trait must be cheap to freeze: schema declared on the
  trait (not optional), JSON in / JSON out matching MCP's wire format.

## Open questions

The runner-side questions are answered in TOOLS-PORTING.md. The two
the agent must still resolve in stage T2 before freezing the trait
at the REVIEW gate:

1. Does `ToolCtx` carry `mcp_session: McpSessionHandle` from day one,
   or is it added in Phase 2? TOOLS-PORTING.md leans "include it now
   so Phase 2 doesn't reshape ToolCtx." Confirm with a one-line
   justification in the handover at the first REVIEW.
2. Is the registration mechanism a `inventory`-style static
   registration, an explicit `register_tool(...)` call from a
   crate-level `register()` fn, or a `ToolRegistry::builder()`?
   TOOLS-PORTING.md does not pick. Lean: explicit `register_tool(...)`
   calls from a per-crate `register()` fn — matches how plugins will
   register later (PLUGIN-SUBSTRATE.md item 6) without `inventory`'s
   linker games. Resolve in T2's commit message; record the call in
   the handover.

## Deliverables

- `codeless/crates/codeless-tools/` crate with `Tool`, `ToolCtx`,
  `ToolError`, registration, `testing::fake_ctx()`, `policy::{NetworkMode,
  AllowlistFile}`, `tools::browse::fetch`.
- `codeless-mcp` change advertising the registry's tools.
- `codeless/NOTICE` with moxxy MIT notice.
- `DOCS/SCOPE.md` crate table updated.
- Per-stage commits on branch `codeless/tools-layer`.
- Final commit at the closing REVIEW gate references this SCOPE.md
  and ticks Phase 1 done in TOOLS-PORTING.md.
