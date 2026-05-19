# assistant-fs-tools — stage 3 → done

Stage 3 added the `mode` column to `assistant_threads`, the
`AssistantThreadMode` enum on the wire, and the
`assistant.setThreadMode` RPC. The column defaults to `'read-only'`
so existing rows back-fill to the safest posture, and the wire enum
on `SetAssistantThreadModeArgs::mode` rejects unknown strings before
the handler runs.

## Schema

`crates/codeless-runtime/migrations/0028_assistant_thread_mode.sql`
adds `mode TEXT NOT NULL DEFAULT 'read-only'` via `ALTER TABLE`. No
SQLite `CHECK` constraint — the parsing-time enum
(`AssistantThreadMode::from_wire`) is the gate, which keeps the
migration reversible and sidesteps SQLite's rewrite cost if D1 ever
grows a fourth mode (SCOPE.md Decisions D2). The migrations
appendix-A test (`assistant_tables_match_stage_5_schema`) was
updated to expect the new trailing column, and a new
`assistant_threads_mode_defaults_to_read_only` test inserts a row
without an explicit `mode` and asserts the `'read-only'` default
applies on the upgrade path.

## Wire surface

- `AssistantThreadMode` lives next to `AssistantThread` in
  `codeless-types::assistant`. Three variants — `ReadOnly`,
  `ApproveEdits`, `Bypass` — serialised as kebab-case strings
  (`"read-only"` / `"approve-edits"` / `"bypass"`). Helper methods
  `as_wire()` and `from_wire()` centralise the column-string mapping
  so the migration literal, the codec, and the docs stay in sync.
- `AssistantThread` gained a `mode` field (specta-typed, `#[serde(default)]`
  so older clients deserialising a freshly-written row still parse).
- `SetAssistantThreadModeArgs` / `SetAssistantThreadModeResult` land
  in `codeless-rpc::methods`; `RpcServer::set_assistant_thread_mode`
  is the new trait method. The HTTP route
  (`/rpc/set_assistant_thread_mode`), the Tauri command
  (`rpc_set_assistant_thread_mode`), and the `HttpRpcClient` impl
  are all wired through.
- The generated TS surface (`ui/codeless-ui/src/lib/rpc/generated/wire.ts`,
  plus both `tests/wire.ts.snap` / `tests/wire-rpc.ts.snap` specta
  snapshots) was re-emitted via `cargo run -p codeless-rpc --example
  wire_ts` and `SPECTA_UPDATE=1 cargo test --test specta_snapshot`.

## Runtime

`set_assistant_thread_mode` in `crates/codeless-runtime/src/rpc/assistant.rs`
delegates to the new `SqliteStore::set_assistant_thread_mode`, which
runs a single-column `UPDATE` and returns `bool` so the handler can
distinguish `NotFound` (no row matched) from `unchanged`.
`updated_at` is *not* bumped — a permission flip is not a
conversational event and re-sorting the rail every dropdown toggle
was explicitly rejected (the `set_thread_mode_does_not_bump_updated_at`
test pins this).

`create_assistant_thread` initialises the new row with
`AssistantThreadMode::default()` (= `ReadOnly`); the codec
(`assistant_thread_from_row`) parses the column via `from_wire` and
surfaces an unknown variant as a `sqlx::Error::Decode` rather than
silently falling back.

## Round-trip tests

Five new tests in `rpc::assistant::tests`:

- `create_defaults_mode_to_read_only` — a freshly-minted thread
  lands at `ReadOnly` both on the returned row and on a re-read.
- `set_thread_mode_round_trips_through_all_three_variants` — flips
  through `ApproveEdits → Bypass → ReadOnly` and re-reads via
  `get_assistant_thread` after each step. The result is *not*
  trusted on its own; storage is the source of truth (R4).
- `set_thread_mode_unknown_thread_is_not_found` — `NotFound` is the
  surface the stage-7 UI needs to distinguish "rail disagrees" from
  "mode unchanged".
- `set_thread_mode_does_not_bump_updated_at` — sleeps past the
  `now_ms()` resolution and asserts the timestamp is unchanged.
- `list_threads_surfaces_persisted_mode` — the rail-rendering
  surface round-trips the persisted mode so the dropdown that lands
  in stage 7 reads a fresh value.

## Verify

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test -p codeless-runtime` (lib + tests) — green, including
  the five new assistant-mode tests and the migrations table-shape
  test.
- `cargo test -p codeless-types -p codeless-rpc -p codeless-client
  -p codeless-server` — green; specta snapshots regenerated and
  match.
- Pre-existing env failures (untouched by this stage): the
  `codeless-plugin-host-wasm` e2e tests and one
  `codeless-runtime/tests/plugin_substrate_e2e.rs` test require the
  `wasm32-unknown-unknown` rustc target which is not installed in
  this worktree, and two `codeless-adapters-host` `git_*` tests
  cannot find `git` on the sandboxed test PATH. None of these touch
  the schema, RPC, or store surfaces this stage modified.

## What stage 4 needs to know

- The mode column is in place; stage 4's read-only tools can short
  out write tools by reading `thread.mode` and refusing to register
  `fs.write` / `fs.edit` when it is `ReadOnly` (SCOPE.md D8).
- The migration is non-destructive — re-running the migrator on an
  existing DB ALTERs the column in place.
- The `assistant.setThreadMode` RPC takes the typed enum, so any
  stage-7 UI work can call it without learning the wire strings;
  TypeScript reads them off `AssistantThreadMode` in `wire.ts`.

## Worktree quirk (not in commit)

When this worktree was first opened, `/home/user/.codeless/worktrees/ai-runner`
pointed at the prior job's worktree as its `workspace`. The pointer
was retargeted at this worktree (`workspace = "../job-01KRZA7KWM0JQ1RBH70FV0FM9P"`)
and the locally-checked-out `ai-runner/src/types.rs` was synced from
the canonical copy under `/home/user/code/rust/codeless-workspace/ai-runner`
so `CliCfg::mcp_config_path` resolves again. Neither edit is in
this worktree's git tree; both are environment fixes for verifying
the stage. The next stage's session will inherit the same setup
unless the worktree is rebuilt from scratch.
