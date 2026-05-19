## Done

- Stage 8 committed (`7ae3658`) on `codeless/adapter-registry`.
- Introduced `RunnerConfig { claude, anthropic, codex, copilot }` in `crates/codeless-runtime/src/default_runner_factory.rs` with `from_effective(&EffectiveAdapterRegistry)` + `any_real()`; collapsed `DefaultRunnerFactory.enable_*` into `config: RunnerConfig`. Re-exported from `codeless-runtime::lib`.
- Created `crates/codeless-cli/src/chat_adapter_registry.rs` exposing `ChatAdapterRegistry { slack, telegram }` and `ChatAdapterRegistry::spawn(&effective, &store, rpc)` that lifts the inline Slack/Telegram spawn out of `serve.rs`. Module wired through `main.rs`.
- Refactored `codeless-cli/src/serve.rs` to construct the factory via `RunnerConfig::from_effective(&effective)` and to spawn chat adapters via `ChatAdapterRegistry::spawn(...)`; deleted the two inline `if effective.<kind>_enabled` blocks. `--enable-*` CLI flags continue to upsert rows (unchanged from stage 3).
- Updated `crates/codeless-tauri-desktop/src/boot.rs` to construct `DefaultRunnerFactory` with `config: RunnerConfig { claude: true, anthropic: true, ... }`.
- Updated SCOPE.md deliverable + handover.md for stage 9.
- Verifications: `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --workspace --no-fail-fast -- --test-threads=4` all green. R1 grep clean on touched files.

## Next

- Stage 9 (REVIEW gate): server-side milestones complete. Flag Gmail adapter and stage 2 (hot-reload) as separate follow-up jobs per WORKFLOW.md.

## What you need to know

- ChatAdapterRegistry lives in `codeless-cli` rather than `codeless-runtime` so the runtime keeps zero `codeless-slack` / `codeless-telegram` dependency (R1). RunnerConfig lives in `codeless-runtime` so tests can construct factories without the CLI.
- The registry's two fields carry `#[allow(dead_code)]` because they exist only to keep handles alive for the process lifetime; the allow is per-field (not struct-wide) so future shutdown wiring still surfaces unused additions.
- Diagnostic strings (`codeless-server: slack adapter enabled (channel=…)`, `--enable-<kind> ignored: <err>`) are byte-for-byte the same as before — operator logs unchanged.
- No new `tokio::process` / `std::process::Command` use anywhere outside `codeless-adapters-host`.

## Open questions

- (none)
