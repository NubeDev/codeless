## Done

- Added `tests/fixtures/fuel_loop/` — a detached-workspace cdylib fixture exporting the default `plugin` world, whose `tool.call` is a `core::hint::black_box`-guarded `wrapping_add` loop. Without `black_box` LLVM would collapse the body to `unreachable` and the test would observe `UnreachableCodeReached` instead of `OutOfFuel`.
- Added `plugin_wasm_e2e::wasm_plugin_respects_fuel_cap` (loads the fixture under `HostPolicy { fuel = 100_000, memory = 64 MiB, deadline = 200 ms }`, asserts `ToolCallOutcome::Err { code: "limit-exceeded", message contains "fuel" }`, and that `elapsed < 200 ms`).
- Verified the test runs in ~0.3 s for the full `plugin_wasm_e2e` integration target; all 4 e2e tests + 5 unit + 3 smoke pass; fmt + clippy clean for the crate.
- Committed as `f2c40ba` on `codeless/plugin-substrate-runtimes`.

## Next

- (none — stage 9 will be picked up by a fresh session.)

## What you need to know

- The fuel / memory / deadline plumbing was already in place from stage 4 (`HostPolicy` -> `Store::set_fuel`, `StoreLimits::memory_size`, `tokio::time::timeout` around `call_call`, `classify_trap` mapping `Trap::OutOfFuel` -> `"fuel"` and `Trap::MemoryOutOfBounds` -> `"memory"`). Stage 8 added the test that proves it works, not new runtime code.
- The "manifest accepts but the plugin cannot self-raise limits" half of the stage is already pinned by existing tests: `PluginManifest` (`crates/codeless-tools/src/plugin/manifest.rs`) uses `#[serde(deny_unknown_fields)]` on `PluginCapabilities`, and `manifest::tests::rejects_unknown_capability_field` asserts that `[runtimes.capabilities] fuel = 1` is a parse error. No new code was needed for that half.
- The fuel-loop fixture builds via the same `build_fixture` helper as the other two fixtures (cached under `target-wasm/`, detached `[workspace]` block in its `Cargo.toml`).
- Outer `tokio::time::timeout(5 s, ...)` is slack on purpose — only there as a deadlock guard. The load-bearing budget assertion is `elapsed < 200 ms`, which observes the policy deadline at the test boundary.

## Open questions

- (none)
