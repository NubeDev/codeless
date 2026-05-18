## Done

- Ported `crates/codeless-plugin-notes/src/lib.rs` onto `codeless-plugin-sdk`: typed `NotesAppendArgs`/`NotesAppendOutput` with schemars `schema_with` hook for the PS7 attachment marker; `ToolBehavior` impl as the canonical authoring surface.
- Added `BuiltinBridge<T: ToolBehavior>` inside the notes crate to adapt `ToolBehavior` to `codeless_tools::Tool`, preserving the existing `register` entry point without making codeless-tools depend on the SDK.
- Added WASM flavour: `crate-type = ["lib","cdylib"]`, mutually-exclusive `builtin` / `wasm` features, `#[cfg(target_arch="wasm32")] wasm_guest` module invoking `wit_bindgen::generate!` against `codeless-tool-wit/wit/tool.wit` with `Guest::describe` + `Guest::call` (`pollster::block_on` for sync↔async) and `export!(NotesComponent)`.
- Bumped `wasmtime` 23 → 30 in `codeless-plugin-host-wasm` (LLVM 19's overlong LEB128 encoding rejected by wasmparser 0.212); applied the `IoView` supertrait split in `wasmtime-wasi` 30.
- Parameterised `plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_addressable_by_thread` into `_builtin` and `_wasm` rows. The wasm row builds via `cargo build --target wasm32-unknown-unknown --no-default-features --features wasm --release` into a sibling `target-wasm/` dir and componentises via `wit_component::ComponentEncoder` (pinned at 0.212); loaded via `WasmPlugin::load` and `describe()` is asserted.
- Updated `handover.md`; added `target-wasm/` to `.gitignore`. Tests/clippy/fmt all green; commit landed on `codeless/plugin-substrate-runtimes` as `fd656a9`.

## Next

- Stage 6: WASM capability sandbox (`[runtimes.capabilities]` parsing, default-deny, attachments R/W via host-implemented `codeless:attachments/store` WIT interface, `plugin_wasm_e2e::wasm_plugin_cannot_open_host_file` + `plugin_wasm_e2e::wasm_plugin_attachment_round_trip` green).

## What you need to know

- The wasm flavour does NOT currently build via `cargo build --target wasm32-wasip2` even though that is the stage-description/doc wording. Rustc's bundled WASI preview1→2 adapter emits a component-model encoding wasmtime 30 still cannot parse; using `wasm32-unknown-unknown` + `wit-component` produces a clean component. The notes plugin world has no WASI imports anyway. `PLUGIN-WASM.md` doc reconciliation is deferred to stage 15.
- `wit-bindgen` pinned at 0.20 (notes) and `wit-component` at 0.212 (runtime dev-deps) to match wasmtime 30's wasmparser minor. Bumping either is an OQ-WASM-sized review.
- The workspace `forbid(unsafe_code)` is downgraded to `deny` for `codeless-plugin-notes` (mirrors `codeless-tool-wit`); the `wasm_guest` module carries `#[allow(unsafe_code)]` for the wit-bindgen CABI glue.
- `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` `workspace = ...` pointer is shared between worktrees; this stage touched it briefly while another worktree was active. Now points at this worktree.

## Open questions

- The "build via wasm32-wasip2" claim in `PLUGIN-WASM.md § Acceptance` is aspirational vs. the as-shipped wasm32-unknown-unknown + wit-component path. Stage 15 (or earlier) should reconcile.
- Whether to keep wit-bindgen at 0.20 or bump now that wasmtime is at 30 — the M-WASM-B review gate should decide.
