# adapter-registry — stage 7 → stage 8

Stage 7 landed. `restart_server` is the sixth RPC of stage 1; the
RPC partitions running jobs into resumable / killed, refuses unless
`force = true`, and branches on the three execution contexts
(`SupervisedCli` exits 75 `EX_TEMPFAIL`, `TauriDesktop` exits 0 for
the shell to respawn, `Bare` returns `AdapterError::RestartUnsupervised`
with a copy-pasteable hint). `codeless serve --respawn-on-exit`
self-watcher wraps the bare case so the in-terminal path can opt into
supervised behaviour without external tooling.

## What landed in stage 7

- `crates/codeless-runtime/src/rpc/restart.rs`:
  - `RestartContext` (`SupervisedCli` | `TauriDesktop` | `Bare`).
  - `RestartTrigger` (Arc-shared `Notify` + state mutex; cheap to
    clone; remembers the desired exit code so the CLI's `run_server`
    can return the right `ExitCode` after the listener drains).
  - `EX_TEMPFAIL = 75`, `RESUMABLE_WINDOW = 30s`.
  - `partition_running_jobs` walks `SqliteStore::list_jobs` →
    `list_stages_for_job`; resumable iff
    `is_resumable_runner(runner_id)` (template-driven set: `mock`,
    `anthropic`) AND the latest `started_at` / `ended_at` across the
    job's stage rows is within `RESUMABLE_WINDOW`.
- `RpcServer::restart_server` lives behind the same bearer gate as
  every other RPC; in-process impl in
  `crates/codeless-runtime/src/rpc/restart.rs::restart_server`.
- HTTP wire-up (`crates/codeless-server/src/routes.rs`) + HTTP client
  (`crates/codeless-client/src/http_client.rs`). The route handler
  accepts a missing body (the UI's "restart now" button has no
  payload other than `force=false`) via `Option<Json<RestartServerArgs>>`.
- `crates/codeless-adapters-host/src/respawn.rs`:
  - `SUPERVISED_ENV = "CODELESS_SUPERVISED"` (env-var marker so the
    runtime constructor picks `SupervisedCli`).
  - `supervise(child_argv)` spawns `current_exe()` with the supplied
    argv until the child exits with anything other than 75. The
    `--respawn-on-exit` flag is stripped before the child sees it.
  - R1-gated: this is the only new `std::process::Command` outside
    `codeless-adapters-host` (grep verified).
- `crates/codeless-cli/src/serve.rs`:
  - `--respawn-on-exit` flag wired through `ServeArgs`.
  - `handle()` enters the supervise loop when the flag is set and the
    `CODELESS_SUPERVISED` marker is absent.
  - `run_server()` picks `RestartContext` from env (`CODELESS_SUPERVISED`
    / `INVOCATION_ID` → `SupervisedCli`; `CODELESS_TAURI_SIDECAR` →
    `TauriDesktop`; else `Bare`).
  - `serve_with_extra_shutdown` (new helper in `codeless-server::lib`)
    selects on Ctrl-C *and* the restart trigger; after the listener
    drains the CLI exits with the trigger's `desired_exit_code` so
    the supervisor sees 75.
- `RestartServerArgs` + `RestartServerResult` gained `Default` so the
  route's "no body = default args" path compiles cleanly.
- Re-exports from `codeless-runtime::lib`:
  `RestartContext`, `RestartTrigger`, `EX_TEMPFAIL`, `RESUMABLE_WINDOW`.

### Integration test

`crates/codeless-runtime/tests/restart_server_partition.rs` — 5
tests, all green:

1. `partition_splits_running_jobs_by_runner_and_recency` — seeds
   three Running jobs (recent-mock = resumable, recent-claude =
   killed, stale-anthropic = killed) and asserts the partition.
2. `force_true_proceeds_past_running_jobs` — confirms the trigger
   fires with `EX_TEMPFAIL` under `SupervisedCli` when `force=true`.
3. `bare_context_returns_unsupervised_hint` — bare context refuses
   and the hint mentions `--respawn-on-exit`.
4. `tauri_desktop_context_fires_with_exit_code_zero` — Tauri
   context arms the trigger with exit code 0.
5. `empty_running_set_proceeds_without_force` — no running jobs
   means `force=false` still proceeds.

### Validations run

- `cargo build --workspace` — green.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean after one auto-format pass.
- `cargo test -p codeless-runtime` (incl. the new partition test)
  — green.
- `cargo test -p codeless-types -p codeless-rpc -p codeless-server
  -p codeless-client -p codeless-adapters-host --lib --tests` —
  green (the three pre-existing `git_commit` / `git_diff` cwd-race
  flakes still fail under parallel `cargo test --workspace`, pass
  with `--test-threads=1`; see prior session handover — not in this
  stage's blast radius).

## What stage 8 owns

Lift the Slack / Telegram adapter spawn calls out of
`codeless-cli/src/serve.rs` (currently lines ~570–630) into a
boot-time `ChatAdapterRegistry` driven by the `chat_adapters` table,
and lift the `DefaultRunnerFactory.enable_*` fields into a
`RunnerConfig` driven by the `runner_config` table. The `--enable-*`
flags keep working as upsert shims. No behavioural change beyond the
source of truth.

The validate-cache contract (process-lifetime, cleared on restart)
is now load-bearing for stage 8: after a restart the registry
re-reads SQLite, the cache is empty, and the operator must
re-validate before re-enabling. The decision is recorded in
`DOCS/SCOPE.md` §"Adapter registry, stage 1".

## Pointers for stage 8

- `crates/codeless-cli/src/serve.rs` lines ~570–630 (Slack /
  Telegram spawn) — wrap these in a `ChatAdapterRegistry` that
  iterates `list_chat_adapters` rows.
- `crates/codeless-runtime/src/default_runner_factory.rs` —
  introduce `RunnerConfig` and have `DefaultRunnerFactory` read it
  rather than the four `enable_*` bool fields.
- `crates/codeless-runtime/src/adapter_registry.rs` already exposes
  `load_effective` — keep it as the single boot read.
- R1 boundary: any new `tokio::process` / `std::process::Command`
  outside `codeless-adapters-host` is a build break against the rule
  in `CLAUDE.md`.

## Out-of-scope reminders carried forward

- All UI work (Settings → Adapters page, confirm modal that renders
  the partition) stays in the follow-up UI job.
- `codeless-gmail` crate, OAuth PKCE host wiring, and the
  refresh-token rotation `secrets_changed` event are in the
  follow-up Gmail job.
- Stage 2 (hot-reload via `Arc<ArcSwap<…>>` / `AtomicBool`) stays
  deferred until the trigger conditions in
  `DOCS/WORKSPACE-ATTACH.md` fire.
- The desktop shell's sidecar broker (the code that actually kills
  and respawns the child Tauri-side) stays out of scope; this stage
  shipped only the runtime contract.

## Open questions

- (none for stage 7; the four open questions are resolved in
  `DOCS/SCOPE.md` §"Adapter registry, stage 1").
