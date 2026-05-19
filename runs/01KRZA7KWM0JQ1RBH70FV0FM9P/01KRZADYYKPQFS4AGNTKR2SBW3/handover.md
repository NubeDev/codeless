## Done

- added migration `crates/codeless-runtime/migrations/0028_assistant_thread_mode.sql` adding `mode TEXT NOT NULL DEFAULT 'read-only'` to `assistant_threads`
- added `AssistantThreadMode` enum (`read-only` / `approve-edits` / `bypass`) with `as_wire`/`from_wire` helpers in `crates/codeless-types/src/assistant.rs`; `AssistantThread` gained the `mode` field
- added `SetAssistantThreadModeArgs`/`SetAssistantThreadModeResult` in `crates/codeless-rpc/src/methods.rs`, `RpcServer::set_assistant_thread_mode` trait method, `HttpRpcClient` impl, axum route `/rpc/set_assistant_thread_mode`, and Tauri command `rpc_set_assistant_thread_mode`
- added `SqliteStore::set_assistant_thread_mode` and updated `assistant_thread_from_row` to decode the column; `create_assistant_thread` initialises new rows at `ReadOnly`
- added five round-trip tests in `crates/codeless-runtime/src/rpc/assistant.rs::tests` (default-on-create, three-variant flip, NotFound, no-updated_at-bump, list-surface round-trip) plus `assistant_threads_mode_defaults_to_read_only` in `crates/codeless-runtime/tests/migrations.rs`
- regenerated specta snapshots (`tests/wire.ts.snap`, `tests/wire-rpc.ts.snap`) and `ui/codeless-ui/src/lib/rpc/generated/wire.ts`
- updated `handover.md` and added session doc `DOCS/sessions/2026-05-19-assistant-fs-tools.md`
- committed as `f84db5d` on `codeless/assistant-fs-tools` with the stage-3 title; not pushed

## Next

- stage 4: implement read-only fs tools (`fs.list`, `fs.read`, `fs.search`) in `codeless-tools` with workspace-root path sandbox; register on the planner's registry for assistant threads only; per-tool unit tests + MockRunner integration test

## What you need to know

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and the relevant `cargo test` subsets are green; pre-existing env failures (`wasm32-unknown-unknown` not installed; sandbox PATH lacks `git` for two `codeless-adapters-host` tests) are unrelated to this stage
- the worktree shipped with `/home/user/.codeless/worktrees/ai-runner` pointed at the prior worktree (`workspace = "../job-01KRMRHE..."`) and a stale `src/types.rs` missing `CliCfg::mcp_config_path`; both were fixed locally (workspace pointer retargeted, types.rs synced from `/home/user/code/rust/codeless-workspace/ai-runner`). Neither edit is tracked here. A clean rebuild of the worktree would need the same fix
- the migration uses `ALTER TABLE ADD COLUMN` so the column lands trailing (`id, title, persona_id, created_at, updated_at, mode`); `assistant_tables_match_stage_5_schema` was updated to match
- enforcement is **server-side**: `AssistantThreadMode::from_wire` rejects unknown strings, the codec surfaces a decode error, and the SQLite column has no `CHECK` (SCOPE.md D2). `updated_at` is intentionally not bumped on mode flip (pinned by `set_thread_mode_does_not_bump_updated_at`)
- commit was not pushed; if the loop requires a push (mani-driven), the next runner should run `./bin/mani --config mani.yaml run push --projects codeless` from the workspace root

## Open questions

- (none)
