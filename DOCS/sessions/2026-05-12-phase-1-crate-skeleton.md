# Build status — Phase 1 crate skeleton

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: `CLAUDE.md` (repo root).
> 2. **One logical batch per tick.** Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3: up to 4 contiguous S in one
>    area, OR 1 M (+ optional related S), OR the next sub-stage of an L.
>    Verify + commit + push **each stage** via mani before moving to the
>    next stage in the batch.
> 3. **You MUST schedule the next tick before exiting** — call
>    `CronCreate` with `recurring: false` for a single fire ~1 min from
>    now. If all stages are `[x]`, report `DONE` instead. If you cannot
>    schedule, **do NOT exit silently** — tell the user which stage
>    finished, exactly why scheduling failed, and how to re-kick. See
>    JOB-LOOP.md "If you cannot schedule".
> 4. Update this file in the **same commit** as the code change.
> 5. ⛔ **COMMIT _AND_ PUSH BEFORE THE TICK ENDS.** Pushing is not
>    optional and not "later". A tick that ends with unpushed commits
>    means the next tick (or the next agent, after `/clear` or a fresh
>    session) sees stale remote state and can clobber or duplicate work.
>    `./bin/mani --config mani.yaml run commit --projects codeless` then `mani run push --projects
>    codeless` — both, every tick, no exceptions. If push fails, mark
>    the stage `[!]` and halt. Never `--force`, never `--no-verify`.
> 6. ⛔ **CODE COMMENTS ARE LOAD-BEARING — WRITE THEM CAREFULLY.**
>    Comments are how the *next* AI agent (and the next human) understands
>    intent. Rules:
>    - Explain **why**, not what. The code already says what.
>    - **No emojis.** Anywhere. Ever.
>    - **No task-status comments.** Never reference stages, ticks,
>      milestones, "added in stage 3", "TODO from M5", "fixed for ticket
>      X". Comments describe the code as it stands, not the task that
>      produced it.
>    - **Long-term framing.** Write for someone reading this in 6 months
>      with zero context — invariants, constraints, why this approach
>      over the obvious one.
>    - **Normal length.** A short line where one helps. A short paragraph
>      where the *why* is genuinely subtle. No multi-paragraph essays,
>      no decorative banners, no ASCII art.
> 7. ⛔ **CROSS-PLATFORM REACH IS ENFORCEABLE.** Stages that touch Rust
>    crates respect the iOS-safe / Android-safe columns in
>    `DOCS/SCOPE.md` "Crate layout". Stages that touch UI modules import
>    only `RpcClient` — never `@tauri-apps/api/core` directly. Trip
>    either rule → mark stage `[!]` and halt.

File: DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md
Goal: Land Phase 1 — types, in-process RPC, specta codegen, initial sqlx
      migration, runtime state machine with MockRunner, tracing baseline,
      codeless/CLAUDE.md, secrets CLI, worktree manager, and a green
      end-to-end `codeless run --once` against a runner — all pushed.
Started: 2026-05-12
Last tick: 2026-05-12 (tick 7 — stages 10+11)  DONE
Current stage: 11 / 11

Repo:        codeless
Branch:      feat/bootstrap-cargo-workspace
Memory policy: compact every 3 stages
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag is mandatory.
`L` stages must be split into S/M sub-stages before being worked.

- [x] 1. [S] codeless-types: Repo/Job/Stage/Task/Event/Review structs (serde)
- [x] 2. [M] codeless-rpc trait + in-process implementation
- [x] 3. [S] specta wire-type generation + snapshot test
- [x] 4. [S] sqlx initial migration matching SCOPE.md Appendix A
- [x] 5. [M] codeless-runtime state-machine skeleton + MockRunner test harness
- [x] 6. [S] tracing-subscriber JSON-to-stdout baseline
- [x] 7. [S] codeless/CLAUDE.md at repo root capturing the rules from SCOPE.md
- [x] 8. [S] codeless secrets set/get/rm/list against chmod 600 secrets.toml
- [x] 9. [S] Worktree manager: git worktree add/remove + reaper-on-startup
- [x] 10. [M] codeless run --once --repo <r> "<prompt>" end-to-end against
         a chosen runner, streaming events to stdout
- [x] 11. [S] Phase 1 wrap-up: README pointer, CODELESS.md memory update,
         confirm cargo test --workspace + clippy -D warnings + fmt --check green

Likely batching (planning hint, not a contract):
- Tick 1: stage 1 (S) — types only, isolated, low risk; gives later stages something to import.
- Tick 2: stage 2 (M) — RPC trait + in-process impl.
- Tick 3: stages 3 + 4 (2×S, both wire/schema-adjacent).
- Tick 4: stage 5 (M) — runtime + MockRunner.
- Tick 5: stages 6 + 7 (2×S, mechanical).
- Tick 6: stages 8 + 9 (2×S, both adapters-host adjacent).
- Tick 7: stage 10 (M).
- Tick 8: stage 11 (S) — wrap-up.

Note: bootstrap stage "Cargo workspace + crate stubs" is already complete
(landed in the bootstrap loop). Workspace stub branch is the base for
Phase 1 — see commit ebd18a5.

## Notes
- Stage 11: Phase 1 wrap-up. Inner-repo README now leads with a
  pointer block (CLAUDE.md, CODELESS.md, ../DOCS/SCOPE.md,
  ../DOCS/JOB-LOOP.md) and a quickstart that shows
  `cargo test --workspace`, the three verify gates, `codeless run`,
  and `codeless secrets`. The historical "fork rationale" text is
  preserved under a divider. CODELESS.md "What this repo is, today"
  was rewritten to enumerate the eight crates by their actual Phase 1
  content; durable-facts log gained a Phase 1 completion entry that
  points the next agent at Phase 2 work (real runners, worktree
  threading into `drive_job`, SQLite event log, HTTP/SSE server).
  All three verify gates are green at wrap-up: cargo test --workspace
  passes every test, clippy -D warnings is clean, fmt --check is
  clean.
- Stage 10: `codeless run --repo <path> [--runner mock] "<prompt>"` is
  the end-to-end Phase 1 dogfood path. CLI module split into
  `src/main.rs` (clap definitions + dispatch only), `src/secrets.rs`,
  and `src/run.rs` per the "one concept per file" rule. `run::handle`
  builds an `InProcessRpc`, registers the repo, submits the job,
  subscribes with `EventFilter::Job`, spawns `drive_job` against
  `MockRunner`, and emits each `EventEnvelope` as one JSON line on
  stdout. The drain loop ends on the framing events
  (`job-completed` / `job-failed` / `job-stopped`), not on the
  runner's outcome directly — so the exit code matches what an
  outside observer would see over the wire. Tokio multi-thread
  runtime is built per-invocation rather than via `#[tokio::main]`
  because the secrets subcommand is sync; this keeps the
  `secrets`-only path free of async overhead. `--once` is accepted as
  a placeholder flag (default true) to keep the documented
  invocation shape stable when daemon mode lands. Worktree wiring is
  deliberately out of scope here — the stage title is "streaming
  events to stdout", and the worktree manager from stage 9 will be
  threaded in alongside real (non-mock) runners.
- Stage 9: `WorktreeManager` in `codeless-adapters-host::worktree`
  shells out to `git worktree {add,remove,prune}`. Process spawn lives
  exclusively in this crate — every other crate stays mobile-safe
  (R1). `create(repo, job_id)` adds `<base>/job-<id>` on a new branch
  `codeless/job-<id>` and returns both path and branch in a
  `WorktreeHandle` so callers do not re-derive the names. `remove`
  forces removal then prunes; `reap_orphans` is a startup-time
  `git worktree prune` and is idempotent. `AlreadyExists` rather than
  overwrite — surprise removal of in-flight user work is the
  outcome we want to make impossible. Tests run `git init --initial-
  branch=main` in a tempdir, set `GIT_AUTHOR_*`/`GIT_COMMITTER_*` per
  invocation so CI machines without a global git config still pass.
- Stage 8: `SecretStore` lives in `codeless-adapters-host::secrets`,
  backed by a TOML file (single flat table; `BTreeMap` for stable
  ordering on disk and in `list`). Save is atomic via a sibling `.tmp`
  file with `OpenOptionsExt::mode(0o600)` on Unix so the value is
  never world-readable mid-rename. `set` validates keys
  (no empties, no whitespace or `=`) to keep the on-disk TOML grammar
  unambiguous. The CLI binary is now wired with `clap` derive: subcommands
  `codeless secrets {set,get,rm,list}` + a global `--secrets-file` (also
  `CODELESS_SECRETS_FILE` env). `get` refuses without `--reveal` so an
  accidental `codeless secrets get FOO` does not splash a key on the
  terminal. `set` reads the value from positional, `--from-env NAME`,
  or stdin (rejected if stdin is a TTY); trailing `\n`/`\r\n` from a
  shell pipe are stripped. Tests in
  `codeless-adapters-host/tests/secrets.rs` cover round-trip,
  permissions, key validation, and unknown-key removal; the CLI tests
  live in `codeless-cli/tests/secrets_cli.rs` via `assert_cmd` against
  the actual binary. clap `env` feature was needed for `#[arg(env=…)]`
  to compile.
- Stage 7: inner-repo `codeless/CLAUDE.md` is now the per-repo agent
  contract — distilled from the workspace `CLAUDE.md` plus the
  SCOPE.md crate-layering rules. Designed to be read first by any
  agent that opens the inner repo directly without descending from
  the workspace root. Names R1-R5 (dep direction, comment rules,
  one-file-one-concept, no drive-by, tests-with-code), points back to
  CODELESS.md for durable per-repo memory, defers to the workspace
  CLAUDE.md as the tie-breaker when statements disagree.
- Stage 6: tracing-subscriber JSON layer lives in
  `codeless-runtime::tracing_init`. Two entry points
  (`try_init_json`/`try_init_pretty`) so hosted mode picks JSON for
  systemd/Docker journals and CLI dev picks pretty — per SCOPE.md
  "tracing baseline". Default `RUST_LOG` filter is
  `info,sqlx=warn,hyper=warn` so a quiet shell sees job/stage/task
  transitions without sqlx debug spew. `with_current_span(true)` is
  gated behind the json formatter — must be called after `.json()` or
  it disappears (footgun: silent fall-through to the non-json layer
  builder). `drive_job` carries `#[tracing::instrument(skip_all,
  fields(job_id = %job_id))]` plus `tracing::info!` events at the two
  status transitions so spans actually carry the job_id field SCOPE.md
  promises.
- Stage 5: introduces three new modules in `codeless-runtime`:
  `state_machine.rs` (pure transition guards for Job/Stage/Task —
  returns `TransitionError` rather than panicking so callers can map
  to `RpcError::Conflict`), `runner.rs` (host-side `Runner` trait +
  `RunnerContext`/`RunnerOutcome`; the real `ai-runner` adoption in a
  later phase will replace this surface but the early shape keeps the
  trait object-safe and bus-aware), and `mock_runner.rs` (scripted
  `MockRunner` driving `Vec<MockStep>` of `Emit`/`Sleep`/`Finish`).
  The driver lives in `driver.rs` as a free function `drive_job`
  rather than a method on `InProcessRpc` because the eventual stage-10
  surface composes it with a scheduler — coupling it to the RPC type
  now would force a refactor at that point. State-machine rule:
  framing events (`job-started`, `job-completed`, `job-failed`) are
  emitted only by the driver; runners never publish them. `Stopped`
  is reachable only via the `stop_job` RPC, which races the driver
  through the shared store — the driver re-reads job status after the
  runner returns and silently exits if a stop already landed, so a
  user-initiated stop wins against a completing runner. New runtime
  dep: `thiserror = "1"` for the transition-error type.
- Working branch is feat/bootstrap-cargo-workspace (the existing branch
  that already carries the 8 crate stubs). User confirmed reuse rather
  than cutting a new feat/phase-1-skeleton.
- codeless/CLAUDE.md (stage 7) is distinct from the workspace
  CLAUDE.md at codeless-workspace root — the workspace one already
  exists from the bootstrap loop. SCOPE.md Phase 1 wants one at the
  inner-repo root too.
- Stages 3+4: batched (both S, both wire/schema-adjacent per the
  planning hint). Stage 3 lands `specta` + `specta-typescript` deps in
  `codeless-types` and a single snapshot test
  (`tests/wire.ts.snap`) that re-runs codegen against the checked-in
  TS, with `SPECTA_UPDATE=1` to regenerate intentionally. Real footgun
  discovered: `specta-serde 0.0.10` propagates `#[serde(rename_all =
  "kebab-case")]` to *variant fields*, while serde itself only renames
  the variant discriminant. That made the snapshot disagree with the
  actual JSON wire output (`task-id` vs `task_id`). The `specta`
  macro does **not** forward `rename_all_fields`, so the only way to
  make codegen match serde is to drop container-level `rename_all`
  on the `Event` enum and apply explicit `#[serde(rename = "...")]`
  per variant. That's now the rule: any enum using `tag = "type"` with
  kebab-case wire labels uses per-variant rename, not `rename_all`.
  Wire-format choices recorded in the snapshot: BigInt → JS `number`
  (every i64 we emit fits inside `Number.MAX_SAFE_INTEGER`), ULID
  newtypes specta-mapped to `string`.
  Stage 4 lands `sqlx 0.8` (features: `runtime-tokio + sqlite +
  migrate`) and `migrations/0001_initial.sql` (verbatim Appendix A).
  The `MIGRATOR` is `sqlx::migrate!("./migrations")` — content-hashed
  at build time, forward-only. Tests apply the migrator to
  `sqlite::memory:` and assert all 7 tables, key index names, every
  column name for `repos` and `jobs`, that `tasks` carries
  `depends_on/lease_*`, and that the `events.cursor` autoincrement
  pk hands out 1, 2 in order.
- Stage 2: `codeless-rpc` stays iOS/Android-safe — deps are
  `serde + async-trait + futures-core + thiserror` only. The
  in-process impl is in `codeless-runtime` (host-only), where tokio
  lives. Trait surface: `add_repo / remove_repo / list_repos /
  submit_job / get_job / list_jobs / stop_job / subscribe`. The bus
  uses `tokio::sync::broadcast` with a 1024-event lag tolerance per
  subscriber; a slow subscriber fails its stream rather than back-
  pressuring publishers. `since`-cursor replay returns `Conflict`
  until stage 4 lands the SQLite event log — the test pins this so we
  notice if it silently starts working. clippy gotchas: workspace
  MSRV is 1.78, so `Option::is_none_or` (1.82) is forbidden — use
  `match` instead. `clippy::map_entry` rewrites
  `contains_key + insert` to `Entry::Occupied`. fmt rewraps single-
  line enum variants to multi-line when fields fit awkwardly.
- Stage 1: IDs are ULID newtypes (`RepoId`, `JobId`, `StageId`, `TaskId`,
  `ReviewId`) generated by a small macro to keep the per-type boilerplate
  one line. Money is `CostCents(i64)`, time is `UnixMillis(i64)` — both
  newtype wrappers so the runtime can't accidentally hand a raw `i64` to
  the wrong column. `Event` is `#[serde(tag = "type", rename_all =
  "kebab-case")]` so every variant serializes to the wire labels listed
  verbatim in SCOPE.md "What each level means". `TaskEnqueued` carries
  `depends_on` from day one per SCOPE.md Rule 4. Crate deps: `serde` +
  `ulid` only — must remain iOS/Android-safe.

## Blockers
(none)
