# plugin-substrate-runtimes — stage 3 → stage 4

Stage 3 (codeless-tool-wit scaffold) landed. Stage 4 closes the
WASM-A milestone by adding the host loader `codeless-plugin-host-
wasm` and the runtime-adapter table seam in `codeless-tools`.

## What landed in stage 3

New mobile-safe crate `crates/codeless-tool-wit/` with:

- `wit/tool.wit` — the codeless tool ABI per
  `DOCS/plugins/PLUGIN-WASM.md § The WIT contract`. Declares
  `package codeless:tool@0.1.0`, the `tool` interface (records
  `tool-manifest`, `tool-call`, `tool-error`; variant `tool-result`;
  enum `tier`; functions `describe`, `call`), and a no-IO `plugin`
  world. WASI imports are *not* declared in the default world: stage
  4 wires WASI host-side at instantiation time so the manifest's
  `[runtimes.capabilities]` set decides what crosses the boundary,
  not the WIT.
- `src/bindings.rs` — `wit-bindgen 0.57.1` Rust guest output for
  that WIT, committed in-tree per the OQ-WASM-2 resolution. The
  exact regenerate command is documented in `src/lib.rs`. The
  bindings compile on host targets too because the WASM ABI glue is
  gated behind `#[cfg(target_arch = "wasm32")]`; the pure data
  types (`ToolManifest`, `ToolCall`, `ToolResult`, `ToolError`,
  `Tier`) are reachable on any target.
- `src/lib.rs` — thin re-export module with `TOOL_WIT` and
  `PACKAGE_ID` constants. Carries the rationale for the per-crate
  `unsafe_code = "deny"` override (the workspace-wide `forbid`
  cannot survive contact with `wit-bindgen` guest output;
  hand-written unsafe outside `bindings.rs` is still a review
  failure).
- `tests/smoke.rs` — three doc-only checks: (1) `wit-parser` accepts
  `tool.wit` and confirms the parsed package matches `PACKAGE_ID`;
  (2) `ToolManifest`/`ToolCall`/`ToolResult` round-trip field-by-
  field through the generated types; (3) the `Tier` discriminants
  are pinned to 0/1/2 so a regeneration cannot silently reorder.

The crate is added to the workspace `members` list with a comment
pointing at the OQ-WASM-2 rationale and the regeneration command.

## Verify

- `cargo build -p codeless-tool-wit` — green.
- `cargo test -p codeless-tool-wit` — 3 unit + 1 doc test green.
- `cargo clippy -p codeless-tool-wit --all-targets -- -D warnings`
  — green.
- `cargo test --workspace` — green. The `codeless-adapters-host`
  git tests flaked once when run with the full workspace earlier
  (parallel git-tempdir race, unrelated to this stage); a re-run
  was clean and a `-p codeless-adapters-host --lib` run reproduces
  green.
- `cargo fmt --check` — green.

## Decisions stage 4+ will rely on

1. **`wit-bindgen` 0.57** is the pinned guest generator for v0.1.
   A bump is an ABI-shaped change even when the WIT is byte-stable
   — the committed `src/bindings.rs` diff must be reviewed
   alongside the bump.
2. **No WASI in the default `plugin` world.** Stage 4's host loader
   wires `wasmtime-wasi` against the per-instance capability set
   itself; the WIT does not advertise WASI as a static dependency.
   Companion worlds (`plugin-with-fs`, `plugin-with-http`) only
   land when a plugin needs static-WIT-visible imports beyond
   no-IO.
3. **`TOOL_WIT` and `PACKAGE_ID` constants are the runtime
   introspection surface.** Stage 4's host-side
   `wasmtime::component::bindgen!` reads `wit/tool.wit` directly
   from disk; the constants are for plugin smoke tests and the
   future `codeless plugin show` CLI.
4. **Per-crate lint override is the model** for any future crate
   that has to host a generator output. Don't apply the override
   to a crate that contains hand-written unsafe.

## What stage 4 needs to do

Per `template.yaml`:

> scaffold codeless-plugin-host-wasm (host-only) — Wasmtime engine,
> WASI-p2 component-model linker, per-call instantiation, HostPolicy
> fuel/memory/wall-clock caps; expose a WasmAdapter implementing
> the runtime-adapter trait introduced in this stage in
> codeless-tools

Notes for stage 4:

- The host crate is host-only. R1 + the iOS/Android cargo-check
  matrix is the canary; verify both stay green at the end of stage
  4 (the WORKFLOW.md per-stage discipline §3 commands).
- Stage 4 also introduces the **`RuntimeAdapter` trait + table in
  `codeless-tools`**. Trait stays mobile-safe (no `wasmtime`, no
  `tokio::process` types in the signature). Concrete impls live
  in their host-only crates behind Cargo features. OQ-WASM-1
  resolved this; stage 4 lands the code shape.
- Use `wasmtime::component::bindgen!` from inside
  `codeless-plugin-host-wasm` to generate the *host* bindings
  against `crates/codeless-tool-wit/wit/tool.wit`. Do **not** try
  to reuse the guest `bindings.rs` from this stage — they target
  different ABIs.
- Read `PLUGIN-WASM.md § The crate`, `§ Limits`, and
  `§ Instance lifecycle` cover-to-cover before writing code. Stage
  4 ships the *engine, linker, per-call instantiation, HostPolicy
  caps*; capability sandbox (stage 6) and the e2e tests against
  the notes plugin (stage 5) are separate stages.

## Out-of-scope reminders carried forward

- Estimator plugin and substrate items 2 + 4 stay out of scope.
- `NotesAppend::call` runtime-table writer remains deferred.
- Process runtime is manifest-seam only in stage 13.
- `mcp_forward` is parse-and-fail (stage 14).
- Mobile shell wiring of plugin UI is out of scope.
- The committed `bindings.rs` is never regenerated by `build.rs`;
  always via the documented `wit-bindgen rust …` command (a
  `cargo xtask` task can land in stage 4 if convenient).
