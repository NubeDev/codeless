## Done

- PLUGIN-SUBSTRATE.md per-item Status rows for items 9, 10, 11 rewritten to reflect what landed across stages 2–13: WASM flavour (SDK + WIT + Wasmtime host + capability sandbox + caps); MF UI (plugin-ui-sdk + host wiring + server REST + notes ui/ subtree); process manifest seam (kind=process parses, two-phase scan, loads Failed with structured reason).
- PS-ACCEPT status block extended with table rows for items 9, 10, 11 and the MCP surface, naming the load-bearing integration test in each case. Header updated to "updated 2026-05-18 for plugin-substrate-runtimes".
- CODELESS.md durable-facts entry for 2026-05-18 added: four bullets describing WASM runtime flavour, MF plugin UI (R6), MCP contribution surface, and the process runtime seam — each with the integration tests that pin it.
- Fixed a pre-existing CLI build break inherited from stage 11: `codeless-cli/src/serve.rs` now sets `plugins: None` on `AppState` with a comment pointing at the future `--plugins-dir` wiring (durable fix; the field was added in stage 11 but the CLI never picked it up).
- Final gates green: `cargo test --workspace` (every test passed), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo fmt --check` (clean), `pnpm -C ui/codeless-ui lint` (no-eslint-yet stub exits 0), `pnpm -C ui/codeless-ui test` (126 passed across 25 files in 2.96s).
- Committed as `stage 15: documentation + handover — …` on `codeless/plugin-substrate-runtimes`.

## Next

- (none) — final stage of the plugin-substrate-runtimes job. The branch is ready to merge to master.

## What you need to know

- OQ resolutions across PLUGIN-WASM (1, 2, 4, 5), PLUGIN-UI-FEDERATION (UI-1), and PLUGIN-MCP (MCP-1) were already written in stage 1 with the durable "Resolved 2026-05-18 (plugin-substrate-runtimes stage 1)" markers; this stage did not touch them.
- Deferred surface, summarised: (a) the process plugin host implementation (gRPC supervisor, UDS, circuit breaker) — manifest seam ships today, runtime waits for a polyglot authoring or crash-isolation driver; (b) `mcp_forward` dispatch kind — deferred to v0.2, manifest parses but loads Failed with `"mcp_forward not yet supported"`; (c) WASM `codeless:db/kv@0.1.0` per-plugin SQLite (OQ-WASM-3) — stateless in v0.1, plugins needing state ship as builtin until the WIT is designed; (d) WASM hot reload — refused when migration list changed (OQ-WASM-4), full migration-replay-against-running-server design lives with substrate item 6 / OQ-PS-5; (e) mobile (iOS/Android) MF host wiring — desktop parity is hand-verified, automated browser-shell coverage is what CI exercises today.
- The estimating plugin remains gated on substrate items 2 + 4 (CommonChat extraction + server-side chat state), as the PS-ACCEPT block records — this job did not change that gating.
- ai-runner's `Cargo.toml` `workspace = "../job-..."` pointer is per-worktree state and is not under git in this worktree; nothing to commit there.

## Open questions

- (none)
