# adapter-registry — stage 9 (REVIEW gate) → done

Stage 9 reviewed the full diff from stages 3–8 against the rulebook's
Layer-1 invariants. Gate verdict: **PASS**. The server-side milestones
are complete; Gmail adapter and stage 2 (hot-reload) are flagged as
separate follow-up jobs in `DOCS/WORKSPACE-ATTACH.md`.

## What the gate looked at

- **R1 — crate dependency direction.** New `process::Command`
  call sites added in this job: exactly one, in
  `crates/codeless-adapters-host/src/respawn.rs::supervise`. The
  runtime's `restart_server` RPC fires a `tokio::sync::Notify`
  (`RestartTrigger`) — no process spawn from `codeless-runtime`. The
  CLI's `--respawn-on-exit` path calls into `codeless_adapters_host::
  respawn::supervise` rather than spawning inline. `codeless-types` /
  `codeless-rpc` / `codeless-client` gain no host-only dependencies.
- **R2 — single transport.** All six new RPCs
  (`list_chat_adapters`, `set_chat_adapter_enabled`,
  `validate_chat_adapter_secrets`, `list_runners`,
  `set_runner_enabled`, `restart_server`) are routed in
  `crates/codeless-server/src/routes.rs` inside `rpc_routes` and
  live behind the same `bearer_layer` as every other RPC. No new
  transport channel; SSE event surface is untouched.
- **R4 / R5 — trust boundary, SQLite source of truth.** Migration
  `0027_adapter_registry.sql` adds `chat_adapters((kind,
  instance_id) PK, enabled, configured_at)` and
  `runner_config(runner_id PK, enabled)`. `--enable-*` flags upsert
  these rows; boot reads from the tables via
  `RunnerConfig::from_effective(&EffectiveAdapterRegistry)` +
  `ChatAdapterRegistry::spawn`. Single bearer gates every new RPC;
  `MissingSecrets` is the structured refusal, not a generic Conflict.
  Validate-cache is in-memory (process lifetime) by design — the
  table is the source of truth, the cache is only a "did the
  operator prove these creds during this boot" gate.
- **Wire formats untouched.** `crates/codeless-types/tests/wire.ts.snap`
  diff is additive-only: `AdapterError`, `ChatAdapterRow`,
  `RunnerRow`, `ListChatAdaptersResult`, `ListRunnersResult`,
  `SetChatAdapterEnabledArgs`, `SetRunnerEnabledArgs`,
  `ValidateChatAdapterSecretsArgs/Result`,
  `ChatAdapterSecretProblem`, `RestartServerArgs/Result`,
  `ChatAdapterKind`. Zero removals; every pre-existing TypeScript
  binding is byte-identical.

## New RPC methods (server-side)

1. `list_chat_adapters() -> ListChatAdaptersResult`
2. `set_chat_adapter_enabled(SetChatAdapterEnabledArgs) -> ()` —
   refuses with `AdapterError::MissingSecrets` until a successful
   `validate_chat_adapter_secrets` for the same `(kind, instance_id)`
   lands in this boot's cache.
3. `validate_chat_adapter_secrets(args) -> ValidateChatAdapterSecretsResult`
   — 5s hard timeout, per-`(kind, instance_id)` 5/s sliding-window
   rate limit. Probe is pluggable (`StaticValidationProbe` for tests,
   HTTP-backed probe at boot for the CLI).
4. `list_runners() -> ListRunnersResult`
5. `set_runner_enabled(SetRunnerEnabledArgs) -> ()`
6. `restart_server(RestartServerArgs) -> RestartServerResult` —
   three branches keyed on `RestartContext::{SupervisedCli,
   TauriDesktop, Bare}`; partitions running jobs into
   resumable / killed and refuses with `RestartHasRunningJobs`
   unless `force: true`.

## What is intentionally deferred

- **Settings → Adapters UI.** The closed set of RPCs above is the
  surface the follow-up UI job sits on. No `ui/codeless-ui/` files
  changed in this job (R3, off-limits per WORKFLOW.md).
- **Gmail adapter (`codeless-gmail`).** The `ChatAdapterKind` enum
  deliberately omits `Gmail`; the variant lands paired with the new
  crate, its OAuth PKCE host wiring, and the
  `users.history.list` long-poll. `DOCS/WORKSPACE-ATTACH.md` reworded
  to call this a "separate follow-up job" (was "separate milestone").
- **Stage 2 (hot-reload without restart).** `ChatAdapterRegistry`
  and `RunnerConfig` are the seams a future
  `Arc<ArcSwap<RunnerConfig>>` / per-adapter graceful-shutdown
  story will swap behind; until a documented trigger fires
  (>5 restart-initiated kills/week, mobile lifecycle requirement,
  or a recurring "rotate without dropping" user ask), restart-on-
  apply is the supported path. `DOCS/WORKSPACE-ATTACH.md` now points
  the stage-2 paragraph at this job's seams.

## Validation snapshot

Trusted from stage 8's handover (re-running the full workspace test
matrix is not part of the REVIEW gate's contract):

- `cargo build --workspace` — green.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.
- `cargo test --workspace --no-fail-fast -- --test-threads=4` — green.
- R1 grep — no new `tokio::process` / `std::process::Command`
  outside `codeless-adapters-host`. The only addition is
  `codeless-adapters-host::respawn::supervise`.

## Sentinel

PASS: R1/R2/R4/R5 invariants hold across the stage 3–8 diff —
process spawning stays in `codeless-adapters-host`, every new RPC
rides the bearer-gated HTTP+SSE transport, the new SQLite tables
are the source of truth with the cache as a session-lifetime gate,
and `wire.ts.snap` is additive-only.
