# plugin-substrate-runtimes — stage 5 → stage 6

Stage 5 landed: the `codeless-plugin-notes` plugin is now ported onto
`codeless-plugin-sdk` and compiles as both flavours from one source.
The substrate e2e test
`plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_addressable_by_thread`
is parameterised over both flavours; both green.

## What landed in stage 5
-
- `crates/codeless-plugin-notes/src/lib.rs` rewritten on top of the
  SDK's `ToolBehavior` trait. `NotesAppend` now carries typed
  `NotesAppendArgs` / `NotesAppendOutput` driven by schemars. The PS7
  attachment marker (`{"$ref": "codeless://attachment"}`) is emitted
  by a schemars `schema_with` hook so the schema reaches
  `/properties/attachment/$ref` directly (no `allOf` wrapper).
- Builtin bridge: a generic `BuiltinBridge<T: ToolBehavior>` inside
  the notes crate adapts `ToolBehavior` to `codeless_tools::Tool`,
  preserving the existing `pub fn register(sink: &mut PluginToolSink)`
  entry point. Lives in the plugin (not in `codeless-tools`) so the
  SDK -> host dep direction stays one-way.
- WASM flavour: `Cargo.toml` declares `crate-type = ["lib", "cdylib"]`
  plus mutually-exclusive `builtin` / `wasm` features. The
  `#[cfg(all(target_arch = "wasm32", feature = "wasm"))]` `wasm_guest`
  module invokes `wit_bindgen::generate!` against
  `crates/codeless-tool-wit/wit/tool.wit`, implements `Guest::describe`
  / `Guest::call` (via `pollster::block_on`), and `export!`s
  `NotesComponent`. Crate-level `unsafe_code = "deny"` exception
  mirrors `codeless-tool-wit`; module carries `#[allow(unsafe_code)]`.
- `wasmtime` bumped 23 -> 30 in `codeless-plugin-host-wasm`. LLVM 19
  (rustc >= 1.85) emits overlong LEB128 encodings of memory index 0
  that wasmparser 0.212 rejects with "zero byte expected"; wasmtime 27+
  accepts them. Only API delta was the `IoView` supertrait split in
  wasmtime-wasi 30 — fixed by splitting the `WasiView` impl.
- Parameterised e2e test
  (`crates/codeless-runtime/tests/plugin_substrate_e2e.rs`):
  - Builtin row: existing logic; additionally asserts the PS7 marker
    via the host `ToolRegistry`'s `output_schema()`.
  - Wasm row: a `OnceLock`-cached helper builds the notes plugin via
    `cargo build --target wasm32-unknown-unknown --no-default-features
    --features wasm --release` into a sibling `target-wasm/` directory
    (so the in-flight host `cargo test` lock is undisturbed), then
    composes the core module into a WASI-p2 component using
    `wit_component::ComponentEncoder` (pinned at 0.212). The encoded
    component is loaded via `WasmPlugin::load`; `describe()` is
    asserted to return one manifest with id `notes.append`, tier
    `write`, and the PS7 marker.
  - `CODELESS_NOTES_WASM` env var lets CI point at a pre-built artefact.

### Notable deviation from the stage description

The stage description says "WASM via `cargo build --target wasm32-
wasip2`". The wasm32-wasip2 target ships rustc-bundled WASI preview
1-to-preview 2 adapter glue that produces a component-model encoding
wasmtime 30 still rejects (the embedded adapter module triggers an
offset error). The notes plugin's `world plugin { export tool; }`
has no WASI imports, so the e2e test builds via
`wasm32-unknown-unknown` and componentises with `wit-component` —
the resulting artefact loads cleanly. Stage 15 should reconcile
`PLUGIN-WASM.md`.

### Validations run

- `cargo build -p codeless-plugin-notes` (builtin, default features)
- `cargo build -p codeless-plugin-notes --target wasm32-unknown-unknown
  --no-default-features --features wasm --release`
- `cargo test -p codeless-plugin-notes` — 4 unit + 1 smoke
- `cargo test -p codeless-runtime --test plugin_substrate_e2e` —
  7 tests including the two flavour rows
- `cargo test --workspace --lib --tests --exclude codeless-server` — all green
- `cargo clippy -p codeless-plugin-notes -p codeless-plugin-host-wasm
  -p codeless-plugin-sdk -p codeless-runtime --all-targets -- -D warnings`
- `cargo fmt --check`

## What stage 6 owns

`PLUGIN-WASM.md § Capability sandbox`. Default-deny capability set,
`[runtimes.capabilities]` manifest parsing, attachments R/W via the
host-implemented `codeless:attachments/store` WIT interface,
`plugin_wasm_e2e::wasm_plugin_cannot_open_host_file` +
`plugin_wasm_e2e::wasm_plugin_attachment_round_trip` green.

## Pointers for stage 6

- `crates/codeless-plugin-host-wasm/src/runtime.rs` is the
  `WasiCtxBuilder` site — today builds the deny-everything ctx. Stage
  6 grants the capability set the plugin's `[runtimes.capabilities]`
  block lists.
- `crates/codeless-tools/src/plugin/manifest.rs` is the
  `plugin.toml` parser. Stage 6 extends it with
  `[[runtimes]] [runtimes.capabilities]`. (Stage 13 lands the rest of
  `[[runtimes]]` parsing; stage 6 only needs the capabilities subset
  for its tests.)
- `crates/codeless-tool-wit/wit/tool.wit` is the load-bearing ABI.
  Adding `codeless:attachments/store@0.1.0` is an ABI change; bump
  the WIT, regenerate `crates/codeless-tool-wit/src/bindings.rs`,
  carry the rationale in the head comment.
- The wasm e2e tests will need a fixture plugin (intentionally
  file-opener). Mirror the `target-wasm/` pattern the stage-5 test
  uses for build caching.

## Open questions

- The "build via wasm32-wasip2" claim in `PLUGIN-WASM.md § Acceptance`
  is aspirational; the e2e test uses wasm32-unknown-unknown +
  wit-component. Stage 15 (or earlier) needs a doc reconciliation.
- Whether to keep wit-bindgen pinned at 0.20 or bump now that wasmtime
  is at 30 — the M-WASM-B gate should decide.

## Out-of-scope reminders carried forward

- Estimator plugin and substrate items 2 + 4 stay out of scope.
- `NotesAppend::call` runtime-table writer remains deferred.
- Process runtime is manifest-seam only in stage 13.
- `mcp_forward` is parse-and-fail (stage 14).
- Mobile shell wiring of plugin UI is out of scope.
- The committed `crates/codeless-tool-wit/src/bindings.rs` is never
  regenerated by `build.rs`; always via the documented `wit-bindgen
  rust …` command.
