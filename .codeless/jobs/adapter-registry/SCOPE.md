# Scope — adapter-registry (server-side, stage 1 of the TODO)

The full design is **[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
§"TODO — adapter registry (chat adapters + AI runners)"** at the
bottom of that file. This brief is the trimmed per-job scope; the
authoritative doc wins on any disagreement. The TODO section will
graduate to its own `SCOPE-ADAPTER-REGISTRY.md` when stage 1 ships.

## Goal

Land **stage 1** of the adapter-registry TODO on `master` via the
`codeless/adapter-registry` branch. After this job, a running server:

1. Reads the enabled set of **chat adapters** (Slack, Telegram) and
   **AI runners** (`claude`, `anthropic`, `codex`, `copilot`) from
   SQLite — not from `--enable-*` CLI flags. The flags stay as
   idempotent upsert shims so `setup/init-session.sh` keeps working.
2. Exposes six new RPCs (`list_chat_adapters`,
   `set_chat_adapter_enabled`, `validate_chat_adapter_secrets`,
   `list_runners`, `set_runner_enabled`, `restart_server`) behind the
   bearer gate so the UI can drive the Settings → Adapters page.
3. Restarts itself gracefully on apply, with `RestartHasRunningJobs`
   surfacing which jobs are *resumable* (recent checkpoint) vs
   *killed* (mid-PTY-stream) before the user clicks through. The
   bare-CLI case gets `codeless serve --respawn-on-exit` so it Just
   Works without external supervision.
4. Backs secrets with a `SecretBackend` trait: the existing XDG TOML
   stays as the default; the `keyring`-crate backend is available
   under a config flag.

The UI (Settings → Adapters page) and the Gmail adapter are **not**
touched in this job — both are follow-up jobs. Stage 2 (hot-reload
without restart) is also a separate, later job per the doc.

## In scope

- The four open questions in §"Open questions" resolved with
  reasoning recorded in this file (stage 1).
- `chat_adapters` table with composite PK `(kind, instance_id)`,
  `enabled BOOLEAN NOT NULL`, `configured_at INTEGER NOT NULL`
  (UnixMillis). Default `instance_id = "default"` so today's
  one-instance-per-kind case is a zero-arg row.
- `runner_config` table with PK `runner_id TEXT`, `enabled BOOLEAN`.
  One row per built-in runner (`claude`, `anthropic`, `codex`,
  `copilot`); `mock` stays gated on `real_runner_enabled()` and is
  not represented as a row.
- Boot-time read in [`codeless-cli/src/serve.rs`](../../../crates/codeless-cli/src/serve.rs)
  replacing the `args.enable_slack` / `args.enable_telegram` /
  `DefaultRunnerFactory::enable_*` constructor wiring. `--enable-*`
  flags become idempotent upserts; missing flag = leave existing row
  alone; first boot with the flag = insert `enabled=true` row.
- Wire types in `codeless-types`:
  - `ChatAdapterKind` (`Slack` | `Telegram`; `Gmail` deliberately
    omitted — that's the follow-up crate).
  - `ChatAdapterRow { kind, instance_id, enabled, configured_at }`.
  - `RunnerRow { runner_id, enabled }`.
  - `ListChatAdaptersResult`, `ListRunnersResult`.
  - `SetChatAdapterEnabledArgs { kind, instance_id, enabled }`,
    `SetRunnerEnabledArgs { runner_id, enabled }`.
  - `ValidateChatAdapterSecretsArgs { kind, instance_id }`,
    `ValidateChatAdapterSecretsResult { ok, problems }`.
  - `RestartServerArgs { force: bool }`,
    `RestartServerResult` (success path is "you will not see this
    response; the connection drops").
  - `AdapterError` enum: `MissingSecrets { keys: Vec<String> }`,
    `ValidationFailed { reason: String }`,
    `RestartUnsupervised { hint: String }`,
    `RestartHasRunningJobs { resumable: Vec<JobId>, killed: Vec<JobId> }`,
    `Conflict`, `NotConfigured`.
- Six RPC methods implemented end-to-end with the validate-before-
  enable coupling: `set_chat_adapter_enabled(true)` MUST refuse with
  `MissingSecrets` / `ValidationFailed` unless a prior successful
  `validate_chat_adapter_secrets` for the same `(kind, instance_id)`
  is cached for this session (cache lifetime is one open question).
- Per-`(kind, instance_id)` rate limit (5/s) on
  `validate_chat_adapter_secrets`. 5s hard timeout on each upstream
  validation call (Slack `auth.test`, Telegram `getMe`).
- `SecretBackend` trait with two backends:
  - `TomlFileBackend` (today's `SecretStore`, XDG path) — default.
  - `KeyringBackend` (`keyring` crate) — opt-in via a config flag
    (`--secrets-backend keyring`); per-key entries; falls back to
    TOML cleanly on platforms where Secret Service isn't running.
  The TOML file stays the source of truth for CI / fixtures
  (`--secrets-file <path>`).
- [x] `restart_server` RPC with three branches (stage 7):
  - **Supervised CLI** (`init-session.sh`, systemd): exit 75
    `EX_TEMPFAIL`; the wrapper / unit file re-execs.
  - **Tauri desktop**: returns success; the desktop shell kills its
    `codeless serve` sidecar and respawns it. The shell-side handler
    is out of scope for this job (the desktop shell already brokers
    sidecar lifecycle); the RPC contract is what's in scope.
  - **Bare CLI without supervisor**: returns `RestartUnsupervised`.
    Add `codeless serve --respawn-on-exit` that wraps the inner
    server in a self-watcher (parent re-execs on exit-75) so the
    bare case becomes the supervised case after a one-flag opt-in.
- `RestartHasRunningJobs` partition: enumerate every job in
  `Running` state. A job is *resumable* if its current stage has a
  checkpoint within the last N seconds (N is an open question);
  otherwise it's *killed*. The verb returns the partition unless
  `force: true`. The UI's confirm modal renders this partition
  before Apply (modal work is in the UI follow-up job, not here —
  but the partition must be correct so the modal has truthful
  inputs).
- [x] Lifting the Slack / Telegram adapter spawn and the
  `DefaultRunnerFactory` config out of `serve.rs` into a boot-time
  `ChatAdapterRegistry` and `RunnerConfig`, both driven by the new
  tables. No behavioural change beyond the source of truth; existing
  adapter and runner code is not refactored.
  _Landed: `crates/codeless-cli/src/chat_adapter_registry.rs` +
  `RunnerConfig` / `DefaultRunnerFactory.config` in
  `crates/codeless-runtime/src/default_runner_factory.rs`._
- Exit tests (from WORKSPACE-ATTACH.md §"Exit tests"):
  1. [x] Write-then-fsync-then-restart ordering: a unit test that
     crashes the process between secrets-write and the restart
     signal proves the on-disk state is durable.
  2. [x] `restart_server` partition: a job with a recent checkpoint is
     reported `resumable` and resumes; a job mid-PTY-stream is
     reported `killed` and the kill is logged.
     _Landed: `crates/codeless-runtime/tests/restart_server_partition.rs`._
  3. [x] `set_chat_adapter_enabled(true)` without a prior successful
     `validate_chat_adapter_secrets` returns the structured
     `MissingSecrets` / `ValidationFailed` error.

## Out of scope

- **All UI work.** The Settings → Adapters page, the confirm modal
  that renders the running-jobs partition, the inline validation
  feedback — all of it is a follow-up job. This job ships the RPC
  surface the UI will sit on.
- **Gmail adapter.** `codeless-gmail` crate, OAuth PKCE host wiring,
  `users.history.list` long-poll, `BotTransport` impl, the
  refresh-token-rotation `secrets_changed` event — separate
  follow-up job per the doc.
- **Stage 2 (hot-reload without restart).** Lifting adapter
  lifecycle into `Arc<ArcSwap<…>>` / `AtomicBool` is explicitly
  deferred until the trigger conditions in the doc fire.
- **Tauri desktop sidecar lifecycle code.** The desktop shell
  already brokers sidecar respawn; this job ships the
  `restart_server` RPC contract, not the shell handler.
- **WASM-plugin adapters.** Adapters remain compile-time
  registrations of crates implementing `BotTransport` from
  `codeless-bot-core`. The doc is explicit: recompile required.
- **Multi-tenant anything.** R5: one bearer token authorises every
  RPC.

## Constraints

- **R1** — `tokio::process` / `std::process::Command` may not appear
  in any crate other than `codeless-adapters-host`. The new RPC types
  live in `codeless-types` (iOS-safe, Android-safe); the
  `restart_server` re-exec path that calls `exec()` lives in
  `codeless-adapters-host` and is invoked from `codeless-cli`.
  `codeless-runtime` does not gain a process dependency.
- **R2** — UI imports only `RpcClient`; this job ships no UI, but
  the wire types in `codeless-types` must be `specta`-derived so the
  follow-up UI job can consume them without hand-written TS.
- **R3** — no per-shell adapter code paths; the Tauri-vs-bare-CLI
  split lives in `restart_server`'s server-side logic plus the
  shell's existing sidecar broker, never in a per-shell adapter
  file.
- **R4** — `chat_adapters` + `runner_config` rows are the source of
  truth. No authoritative in-memory state outside the
  `ChatAdapterRegistry` that mirrors them at boot.
- **R5** — every new RPC behind the same bearer gate. No
  per-adapter scopes.
- **Comments rule (R2 in codeless/CLAUDE.md)** — no task-status
  comments, no emojis, no restatements. Comments earn their keep only
  for *why*.
- `cargo test --workspace` / `cargo clippy --workspace --all-targets
  -- -D warnings` / `cargo fmt --check` all green before each commit.
- MSRV 1.78.

## Deliverables (what "done" looks like)

1. `codeless/adapter-registry` branch with one commit per stage,
   pushed via mani.
2. `cargo test --workspace` green; the three exit tests above all
   exercise their behaviour, not just compile.
3. With `--enable-slack` removed from `codeless serve` and Slack
   left enabled in `chat_adapters`, the adapter still spawns at
   boot. With `--enable-slack` passed once and then removed on the
   next boot, the row stays `enabled=true` and the adapter still
   spawns (flag is upsert, not state).
4. `restart_server` from a `init-session.sh`-supervised server cycles
   the process; from a bare CLI without `--respawn-on-exit` returns
   `RestartUnsupervised`; with `--respawn-on-exit` the parent
   re-execs.
5. `validate_chat_adapter_secrets` returns `ok=true` for a valid
   Slack `auth.test` round-trip against a test fixture, and the
   subsequent `set_chat_adapter_enabled(true)` is accepted. With no
   prior validate call, the same `set_*` returns `MissingSecrets`.
6. `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry" gets its
   stage 1 checkboxes ticked and is annotated with a one-line
   "Landed in `codeless/adapter-registry`" pointer.

## Open questions (resolve in stage 1, before any code)

1. **Composite PK `(kind, instance_id)` or single `kind` PK with a
   later schema change?** Bias from peer review: composite now so
   Slack-personal + Slack-work works without a migration. Decide
   whether to default `instance_id = "default"` or require it
   explicitly at insert time. Record the default behaviour for
   `--enable-slack`.
   - **Decision (record here):**
2. **`--respawn-on-exit` default — on or off for `codeless serve`?**
   Bias: off by default so the flag is opt-in; `init-session.sh`
   passes it explicitly. Decide whether the desktop shell relies on
   `--respawn-on-exit` or its own supervisor (today: its own).
   - **Decision (record here):**
3. **Validate-cache lifetime.** The doc says
   `set_chat_adapter_enabled(true)` requires a prior successful
   `validate_chat_adapter_secrets` "within the current session".
   Define "session" — process lifetime? 10 minutes? Per-bearer?
   Bias: process lifetime, in-memory, cleared on restart. Note the
   UX implication: after a restart, the user must re-validate
   before enabling.
   - **Decision (record here):**
4. **Kill-vs-resumable partition rule.** Define the checkpoint
   recency threshold N seconds beyond which a running job is
   reported `killed` rather than `resumable`. Bias: tie it to the
   stage's last `task_progress` event (template runners write one
   per stage transition); a job with a `task_progress` in the last
   30s is `resumable`, older is `killed`. Note that PTY-bound
   runners mid-stream still count as `killed` regardless of the
   timer because the child process dies on restart.
   - **Decision (record here):**

Record the chosen answer + one-line *why* directly under each, then
tick the corresponding line in WORKSPACE-ATTACH.md so the doc and
the job stay in sync.

## References

- Workspace TODO (authoritative): [`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md) §"TODO — adapter registry"
- Project scope: [`DOCS/SCOPE.md`](../../../DOCS/SCOPE.md)
- Agent rules (codeless repo): [`CLAUDE.md`](../../../codeless/CLAUDE.md)
- Agent rules (workspace): [`CLAUDE.md`](../../../CLAUDE.md)
- Existing secrets store: [`crates/codeless-adapters-host/src/secrets.rs`](../../../crates/codeless-adapters-host/src/secrets.rs)
- Existing adapter spawns: [`crates/codeless-cli/src/serve.rs`](../../../crates/codeless-cli/src/serve.rs) lines 500–560
- Existing runner factory: [`crates/codeless-runtime/src/default_runner_factory.rs`](../../../crates/codeless-runtime/src/default_runner_factory.rs)
- Sibling job (same shape): [`.codeless/jobs/workspace-attach/`](../workspace-attach/)
