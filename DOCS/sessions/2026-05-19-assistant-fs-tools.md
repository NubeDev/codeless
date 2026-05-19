# 2026-05-19 — assistant-fs-tools

Job [`assistant-fs-tools`](../../.codeless/jobs/assistant-fs-tools/SCOPE.md)
gives the in-app Assistant a filesystem read/write surface gated by
a per-thread permission mode (`read-only` / `approve-edits` /
`bypass`). This session covers stage 3 — the schema and RPC seam
that everything else hangs off.

## Stage 3 — schema + `setThreadMode` RPC (done)

- Added `mode` column to `assistant_threads` via
  migration `0028_assistant_thread_mode.sql`. `TEXT NOT NULL
  DEFAULT 'read-only'` so existing rows back-fill to the safest
  posture. No SQLite `CHECK`; the server-side enum parses the
  column.
- Added `AssistantThreadMode` to `codeless-types::assistant` with
  three variants (`ReadOnly`, `ApproveEdits`, `Bypass`) and
  `as_wire` / `from_wire` helpers. The `AssistantThread` row carries
  the new field.
- Added `SetAssistantThreadModeArgs` / `Result` to
  `codeless-rpc::methods`, `RpcServer::set_assistant_thread_mode`
  trait method, HTTP route `/rpc/set_assistant_thread_mode`,
  Tauri command `rpc_set_assistant_thread_mode`, and the
  `HttpRpcClient` impl.
- Added `SqliteStore::set_assistant_thread_mode` (single-column
  `UPDATE`, returns `bool`). The handler in
  `codeless-runtime::rpc::assistant` returns `NotFound` when the
  thread is missing; `updated_at` is not bumped.
- Five round-trip tests in `rpc::assistant::tests`: default,
  all-three-variant flip, `NotFound`, `updated_at` invariant, and
  list-surface round-trip.
- Specta snapshots regenerated (`SPECTA_UPDATE=1 cargo test`);
  `cargo run -p codeless-rpc --example wire_ts` re-emitted the
  TypeScript bundle.

### Verify

- `cargo fmt --check` green.
- `cargo clippy --workspace --all-targets -- -D warnings` green.
- `cargo test -p codeless-runtime` green (410 lib + per-test-bin
  greens including the five new ones and the migration appendix-A
  test).
- `cargo test -p codeless-types -p codeless-rpc -p codeless-client
  -p codeless-server` green.

Pre-existing environment failures (wasm32-unknown-unknown target
missing, sandbox PATH without `git`) are unrelated to this stage.

### What lands next (stage 4)

Read-only `fs.list` / `fs.read` / `fs.search` tools in
`codeless-tools`, registered on the planner's tool registry for
assistant threads only. Stage 4 reads the thread row server-side
on every dispatch (SCOPE.md "Constraints" — "UI hints, server
enforces") and uses the `AssistantThreadMode::ReadOnly` check to
short out write-tool registration when stage 6 lands.
