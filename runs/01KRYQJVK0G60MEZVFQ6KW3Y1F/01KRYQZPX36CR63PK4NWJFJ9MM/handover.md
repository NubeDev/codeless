## Done

- Added migration `0027_adapter_registry.sql` with `chat_adapters((kind, instance_id) PK, enabled, configured_at)` and `runner_config(runner_id PK, enabled)`.
- New module `crates/codeless-runtime/src/adapter_registry.rs` with `upsert_chat_adapter` / `upsert_runner` / `list_chat_adapters` / `list_runners` / `load_effective`, registered in `lib.rs`.
- Reworked `codeless-cli/src/serve.rs` so `--enable-slack|telegram|claude|anthropic|codex|copilot` upsert rows at boot and the rest of the serve path (claude scrub, `DefaultRunnerFactory`, Slack/Telegram spawns, `build_server_info`) reads from the table-backed `EffectiveAdapterRegistry`. DB-read failure logs and falls back to the raw flags.
- Migration test `adapter_registry_tables_match_workspace_attach_todo` asserts column order, composite PK, in-place upsert, and `load_effective` round-trip; updated `migrator_creates_all_tables_from_appendix_a` to include both new tables.
- Committed as `deb93d9` on `codeless/adapter-registry` with message starting "stage 3: ...".

## Next

- (none) — stage 4 is for a fresh session.

## What you need to know

- The shared `/home/user/.codeless/worktrees/ai-runner/Cargo.toml`'s `workspace = "..."` pointer is rewritten by each worktree; I temporarily pointed it at this worktree to compile, then restored its original value pointing at `job-01KRYN9FVQ7V3K8EF0XXQGZ45E`. Future jobs that build will keep rewriting it — expect contention if jobs run concurrently.
- `cargo clippy --all-targets` is blocked by a pre-existing missing `compose_system_prompt` test in `serve.rs` (unrelated to this stage; introduced by an earlier commit). I ran `cargo clippy -p codeless-runtime -p codeless-cli -- -D warnings` (no `--all-targets`) clean and the migrations test suite all-green.
- `load_effective` only projects `default`-instance rows for chat adapters today; multi-instance plumbing lands when the registry RPC ships.
- `mock` deliberately has no `runner_config` row — keeping `DefaultRunnerFactory`'s "mock only when no real runner enabled" invariant intact.

## Open questions

- (none)
