# adapter-registry — stage 8 → stage 9 (REVIEW gate)

Stage 8 landed. The Slack and Telegram adapter spawn no longer lives
inline in `codeless-cli/src/serve.rs`; the four `enable_*: bool`
fields on `DefaultRunnerFactory` are gone too. Both groups are now
fed from the SQLite tables (`chat_adapters`, `runner_config`) via a
single boot-time projection.

## What landed in stage 8

- `crates/codeless-runtime/src/default_runner_factory.rs`:
  - New `RunnerConfig { claude, anthropic, codex, copilot }` struct
    with `RunnerConfig::from_effective(&EffectiveAdapterRegistry)` and
    `any_real()`.
  - `DefaultRunnerFactory.enable_*` fields collapsed into a single
    `config: RunnerConfig`. Match arms (`"claude" if self.config.claude
    => ...` etc.) walk the same set; `real_runner_enabled()` now reads
    `self.config.any_real()`.
- `crates/codeless-runtime/src/lib.rs`: re-exports `RunnerConfig`
  alongside `DefaultRunnerFactory`.
- `crates/codeless-cli/src/chat_adapter_registry.rs` (new):
  - `ChatAdapterRegistry { slack, telegram }` holds the spawned bot
    handles for the process lifetime.
  - `ChatAdapterRegistry::spawn(&effective, &store, rpc)` walks the
    closed `(slack, telegram)` set the effective registry exposes
    today and spawns one background task per enabled row. Missing
    secrets and per-adapter init failures keep producing the same
    `--enable-<kind> ignored: <reason>` warnings the inline code did,
    so operators see no diagnostic regression.
  - Lives in `codeless-cli` because the runtime crate does not (and,
    per stage 8 scope, should not) depend on `codeless-slack` /
    `codeless-telegram`; R1 isolation is preserved.
- `crates/codeless-cli/src/serve.rs`:
  - The two `if effective.<kind>_enabled { ... }` blocks are replaced
    by `let _chat_adapters = ChatAdapterRegistry::spawn(&effective,
    &store, state.rpc.clone());` (one statement, identical lifetime
    semantics).
  - `DefaultRunnerFactory` is constructed with
    `config: RunnerConfig::from_effective(&effective)` — the same set
    of bits the old four `enable_*` literals carried, sourced through
    the table.
  - `--enable-slack` / `--enable-telegram` / `--enable-claude` /
    `--enable-anthropic` / `--enable-codex` / `--enable-copilot` all
    keep working as idempotent upsert shims via
    `codeless_runtime::adapter_registry::upsert_chat_adapter` /
    `upsert_runner` (no change from stage 3).
- `crates/codeless-tauri-desktop/src/boot.rs`: the desktop shell
  constructs `DefaultRunnerFactory` with `RunnerConfig { claude:
  true, anthropic: true, codex: false, copilot: false }` instead of
  the four literal bool fields. Same enabled set as before.

## Behaviour

- `--enable-slack` removed from `codeless serve`, row left
  `enabled=true` in `chat_adapters`: adapter still spawns at boot
  (`ChatAdapterRegistry::spawn` reads the row, the inline conditional
  is gone). Same shape for Telegram.
- `--enable-claude` removed, `runner_config.claude.enabled = true`:
  factory still builds `ClaudeRunnerAdapter` for `runner: "claude"`
  jobs (the match arm reads `self.config.claude`).
- A flag re-passed on a later boot still wins (upsert behaviour from
  stage 3 is unchanged); the table is consulted *after* the upsert so
  the flag is a one-shot bootstrap, not state.

## Validations run

- `cargo build --workspace` — green.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
  (one transient `dead_code` on `ChatAdapterRegistry.{slack,
  telegram}` is silenced field-by-field with `#[allow(dead_code)]`
  rather than struct-wide so future fields keep warning).
- `cargo fmt --check` — clean after one auto-format pass.
- `cargo test --workspace --no-fail-fast -- --test-threads=4` —
  fully green.
- R1 grep on stage-touched files (`crates/codeless-cli/src/chat_adapter_registry.rs`):
  no `tokio::process` / `std::process::Command` introductions.

## What stage 9 owns

REVIEW gate. Server-side milestones complete; the registry surface
the UI follow-up consumes is now end-to-end:

- Boot reads `chat_adapters` / `runner_config` (stage 3).
- Six RPCs end-to-end with validate-before-enable coupling (stages
  5–6).
- `restart_server` partitions and re-execs (stage 7).
- Slack / Telegram spawn and runner factory both source-of-truth on
  SQLite, with `--enable-*` flags reduced to upsert shims (stage 8).

Gmail adapter and stage 2 (hot-reload via `Arc<ArcSwap<…>>` /
`AtomicBool`) remain follow-up jobs per `DOCS/WORKSPACE-ATTACH.md`
§"TODO — adapter registry". The Settings → Adapters UI is a separate
follow-up consuming the wire types stage 5 added to `codeless-types`.

## Out-of-scope reminders carried forward

- All UI work stays in the follow-up UI job.
- `codeless-gmail` crate, OAuth PKCE host wiring, and the
  refresh-token rotation `secrets_changed` event are in the
  follow-up Gmail job.
- Stage 2 (hot-reload) stays deferred until the trigger conditions in
  `DOCS/WORKSPACE-ATTACH.md` fire.
- The desktop shell's sidecar broker (the Tauri-side code that kills
  and respawns the child) stays out of scope.

## Open questions

- (none for stage 8; the four open questions are resolved in
  `DOCS/SCOPE.md` §"Adapter registry, stage 1").
