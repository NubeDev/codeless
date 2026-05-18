## Done

- Added `codeless-tools::runtime_adapter` (new module): mobile-safe `RuntimeAdapter` trait (`async_trait`, pure-data `ToolCallRequest`/`ToolCallOutcome`/`AdapterError`/`AdapterToolManifest`), plus `RuntimeAdapterTable` keyed by tool id with collision reporting for `codeless-server`'s two-phase scan. No host-only types cross the trait boundary -- OQ-WASM-1 leak canary stays green.
- New host-only crate `codeless-plugin-host-wasm` (wasmtime 23 + wasmtime-wasi 23, features `runtime,cranelift,component-model,async`):
- `runtime.rs` -- `WasmRuntime` builds the engine with `async_support(true)` + `consume_fuel(true)` and the WASI-async linker prototype; `PluginStoreState` is the default-deny per-call `WasiCtx`/`ResourceTable`/`StoreLimits` carrier.
- `policy.rs` -- `HostPolicy` (defaults 100M fuel / 64 MiB / 10 s per `PLUGIN-WASM.md § Limits`); `HostPolicyOverride::with_override` enforces OQ-WASM-5 "overrides may only lower the cap".
- `plugin.rs` -- `WasmPlugin::load(path)` / `from_component` eagerly compile the component and cache the WIT `describe()` manifests; `call()` does per-call instantiation (`build_store` -> `set_fuel` -> `limiter` -> `instantiate_async` -> `call_call` under `tokio::time::timeout`). Trap classifier maps `OutOfFuel` / `MemoryOutOfBounds` / timeout to `tool-error.code = "limit-exceeded"`.
- `adapter.rs` -- `WasmAdapter` wraps `Arc<WasmPlugin>` and implements `codeless_tools::runtime_adapter::RuntimeAdapter`.
- `bindings` -- host-side `wasmtime::component::bindgen!({ path: "../codeless-tool-wit/wit", world: "plugin", async: true })`. Separate from the in-tree guest bindings (OQ-WASM-2 still holds).
- `error.rs` -- `HostError { Engine, InvalidComponent, LimitExceeded { reason }, GuestViolatedAbi }`.
- Workspace `Cargo.toml` adds the new member with a host-only/OQ-WASM-1 pointer comment.
- Tests: `codeless-tools::runtime_adapter` unit tests (register dispatch, double-registration collision, AdapterError builder); `codeless-plugin-host-wasm` unit tests (garbage bytes -> `InvalidComponent`, trap classifier) plus `tests/runtime_smoke.rs` (engine constructs, defaults match `PLUGIN-WASM.md`, `RuntimeAdapter` object-safe).
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all green. `cargo tree -p codeless-client` shows no `wasmtime` / `codeless-plugin-host-wasm` -- the iOS/Android-safe canary holds.
- Commit `bc3bab7` on `codeless/plugin-substrate-runtimes`.

## Next

- Stage 5: port `codeless-plugin-notes` onto `codeless-plugin-sdk` so the same source builds as both flavours (`cargo build` + `cargo build --target wasm32-wasip2`); parameterise `plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona_addressable_by_thread` over `builtin` and `wasm`; both green. End-to-end "load a real `.wasm` and dispatch a `call`" coverage of the stage-4 host happens here, against a real notes artefact.
- Stage 5 should add a `cargo xtask wit-bindgen` task (promised in `codeless-tool-wit/src/lib.rs`) and the `register!` macro's per-feature expansion (`inventory::submit!` for builtin, `wit-bindgen` `export tool` for wasm).

## What you need to know

- Wasmtime version is pinned to `"23"` -- the doc-cited floor (`PLUGIN-WASM.md § Why a component-model host`: "`wasmtime` 23+"). Local rustc is 1.90.0; workspace MSRV is 1.78. Wasmtime 23 builds on 1.78; v44 demands 1.92, so bumping the floor is an OQ-WASM-* sized decision and must update the doc first.
- `HostPolicy::memory_max_bytes` is `u64` for fail-fast at config parse on a 32-bit host; the per-call `build_store` does `usize::try_from` and returns `HostError::Engine` if the override exceeds `usize::MAX`. `StoreLimitsBuilder::memory_size` takes `usize`, which is why we cast at the boundary rather than at config-load time.
- WASI is default-deny in `PluginStoreState::new`: `WasiCtxBuilder::new().build()` -> no preopens, no env, no stdio inheritance. Stage 6 owns the capability-grant path; until then a plugin importing any non-default WASI interface fails to link, which is the correct posture.
- The trap classifier conservatively maps `Trap::MemoryOutOfBounds` to `"memory"` -- it overlaps with genuine out-of-bounds accesses, but both surface to the agent loop as `limit-exceeded`, so the distinction doesn't matter to the dispatcher. If stage 6/7 needs finer separation, split it then; don't pre-emptively widen the vocabulary.
- The host-side bindgen is regenerated per build; the guest-side bindings stay in-tree under `codeless-tool-wit/src/bindings.rs` per OQ-WASM-2. Two generators, two trees, on purpose.
- Commit made with raw `git` (mani isn't in this worktree). Stage 3 handover noted the same; nothing pushed.
- One transient flake the previous handover mentioned (`codeless-adapters-host` git tests under parallel tempdirs) did not reproduce here; full workspace tests passed first try.

## Open questions

- (none)
