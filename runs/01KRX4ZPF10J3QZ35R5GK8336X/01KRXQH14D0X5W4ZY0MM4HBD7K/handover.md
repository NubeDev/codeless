## Done

- Manifest parser extended (`crates/codeless-tools/src/plugin/manifest.rs`): `PluginRuntime` gains a `binary: Option<PathBuf>` field for the process flavour and a `policy: PluginRuntimePolicy` sub-block (`socket_ready_timeout`, `health_interval`, `failure_threshold`, `failure_window`, `failed_cooldown` — sum-type accepting `false` or a duration string). `validate_runtime_kind_block` enforces required-vs-not-applicable per kind so `binary` on a `builtin` entry, `policy` on a `wasm` entry, or a missing `binary` on a `process` entry all fail at parse. New duration parser (`<int>{ms,s,m}`) validates strings at manifest-load; `ResolvedPluginRuntimePolicy` is the typed mirror callers consume via `.resolve()`.
- Two-phase scanner (`crates/codeless-tools/src/plugin/substrate.rs`, new): `scan_plugins_dir(dir, &RegistrationTable) -> ScanResult` walks a parent directory of plugin subdirs. Phase 1 parses + vets each manifest in isolation (process-only plugins land as `PluginLoadOutcome::Failed` with `PluginFailureReason::ProcessRuntimeNotSupported`, builtin-without-RegistrationTable as `UnknownBuiltin`, parse failures as `Unparseable`); phase 2 only loads vetted plugins into a fresh `PluginRegistry`, so a per-plugin failure cannot half-populate the registry. `resolve_active_runtime` picks builtin > wasm > process when a plugin ships multiple `[[runtimes]]` entries.
- `PluginFailureReason::code()` returns wire-stable kebab-case ("process-runtime-not-supported", "unknown-builtin", "no-loadable-runtime") for future `GET /plugins` JSON projection.
- New e2e test `plugin_substrate_e2e::process_runtime_declared_today_loads_failed_with_structured_reason` writes a real plugin dir with `kind = "process"` + a full `[runtimes.policy]`, runs `scan_plugins_dir`, and asserts the structured Failed outcome, the operator-facing message, the registry is untouched, and the policy round-trips through `.resolve()` as `Duration`s.
- 12 new manifest unit tests + 5 new substrate scanner unit tests. All 126 `codeless-tools` lib tests green; all 8 `plugin_substrate_e2e` integration tests green. `cargo clippy -p codeless-tools -p codeless-runtime --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean.
- Committed as `c34672c` on `codeless/plugin-substrate-runtimes` with the stage title prefix.

## Next

- Stage 14: `mcp_forward` parse-and-fail wiring per PLUGIN-MCP.md OQ-MCP-1 resolved-2026-05-18 (the scanner already has `PluginFailureReason::NoLoadableRuntime` as a sibling slot; a new `McpForwardNotSupported` reason can land next to `ProcessRuntimeNotSupported` once the `[contributes.mcp]` parser is added).
- Stage 15: doc reconciliation (the wasm32-wasip2 vs wasm32-unknown-unknown + wit-component story noted in the stage-5 handover).
- A future host crate (`codeless-plugin-host-process`) will consume `ResolvedPluginRuntimePolicy` directly; no field renames will be needed.

## What you need to know

- `PluginManifest::runtimes` is now strict-validated: any plugin that ships a builtin entry without `crate`, a wasm entry without `artefact`, a process entry without `binary`, or mixes kind-specific fields will fail at `from_dir`. The on-disk `plugins/notes/plugin.toml` already conformed, so no plugin file required changes.
- The substrate scanner is in `codeless-tools::plugin::substrate`, not in `codeless-server`. Despite the stage title saying "codeless-server manifest parser", `codeless-server` does not depend on `codeless-tools`; the scanner has to live where `codeless-runtime`, `codeless-cli`, and (future) `codeless-server` can all reach it. Wiring the scanner into the actual server boot path is a follow-up — today the scanner is callable, the e2e test drives it, and `codeless plugin list` will project its outcomes when that hookup lands.
- The shared `../ai-runner/Cargo.toml` has a `workspace = "../job-<id>"` pointer that another worktree (`job-01KRXH0RYTT6EYGF435WPQS70Q`) flipped back during this session. Builds in this worktree need it pointed at `job-01KRX4ZPF10J3QZ35R5GK8336X` to succeed. The committed work compiled cleanly; if a fresh build fails with "wrong workspace", flip the pointer locally — it is a cross-worktree race, not anything checked in.
- `PluginLoadOutcome::Loaded.loaded` is `Box<LoadedPlugin>` to keep `large_enum_variant` clippy happy; the alternative was suppressing the lint, which `-D warnings` forbids.
- `failed_cooldown = true` is explicitly rejected (`BadFailedCooldownLiteral`); only `false` or a duration string are valid, matching the PLUGIN-PROCESS.md example shape.

## Open questions

- The scanner today drives only builtin-flavour plugins through `PluginRegistry::load_plugin` for the commit phase; wasm-active plugins are recorded as `Loaded` with empty `tool_ids` because the wasm host loader runs separately (`codeless-plugin-host-wasm::WasmPlugin::load`). When `codeless-plugin-host-wasm` grows its own adapter-table integration, the scanner will need to thread that loader in too — likely as a `WasmLoader` trait parameter, parallel to `RegistrationTable`.
- The committed `crates/codeless-tool-wit/src/bindings.rs` is never regenerated by `build.rs` (carried forward from prior stage notes); no change in this stage but worth restating before stage 14 touches the ABI.
