# CODELESS.md — project memory

This file is the project's per-repo memory. It captures durable facts
about *this codebase* that survive across sessions, ticks, and agents.
Ephemeral context (current task, current branch state) lives in the
**session files** in the parent workspace at
[`../DOCS/sessions/`](../DOCS/sessions/), not here.

## Where to find things

Codeless lives inside a multi-repo workspace. The parent
[`codeless-workspace`](https://github.com/NubeDev/codeless-workspace)
holds the design docs, the bundled `mani` binary, and the vendored
`ai-runner` crate.

- Project scope, all decisions, the crate table, all open questions:
  [`../DOCS/SCOPE.md`](../DOCS/SCOPE.md)
- Agent-facing rules (R1-R5): [`../CLAUDE.md`](../CLAUDE.md)
- Autonomous build loop spec: [`../DOCS/JOB-LOOP.md`](../DOCS/JOB-LOOP.md)
- Loop kickoff template:
  [`../DOCS/JOB-LOOP-KICKOFF.template.md`](../DOCS/JOB-LOOP-KICKOFF.template.md)
- Multi-repo workflow: [`../DOCS/MANI.md`](../DOCS/MANI.md)
- Active session docs: [`../DOCS/sessions/`](../DOCS/sessions/)

## What this repo is, today

Phase 1 (crate skeleton) landed on `feat/bootstrap-cargo-workspace`.
Phase 2a (persistence + queue) sits on `feat/phase-2a-persistence`:

- `codeless-types` — Repo/Job/Stage/Task/Event/Review structs, serde +
  specta. iOS/Android-safe.
- `codeless-rpc` — `RpcServer` trait, args, results, error variants,
  subscribe surface. iOS/Android-safe.
- `codeless-runtime` — `InProcessRpc` over `SqliteStore` + `EventBus`,
  with the Appendix A schema applied on construction. Events persist
  to the `events` table and the cursor comes from the autoincrement
  column; `subscribe(since)` replays from SQLite and chains to the
  live broadcast tail without gaps or duplicates. Lease-based task
  queue with three-scope concurrency caps (global, per-repo,
  per-runner) lives in `SqliteStore`; `spawn_heartbeat` renews
  leases in a background task and a startup-time reaper inside
  `with_db` reclaims expired leases when the core restarts.
  `Runner` trait + `MockRunner` scripted harness, `drive_job`
  driver, tracing subscriber (`try_init_json` / `try_init_pretty`).
  Host-only.
- `codeless-adapters-host` — `SecretStore` (chmod 0600, atomic-rename
  TOML), `WorktreeManager` (`git worktree add/remove/prune`).
  Host-only; the only crate permitted to spawn processes.
- `codeless-cli` — `codeless run --repo <p> "<prompt>"` end-to-end
  against the mock runner, streaming JSON-line events; `codeless
  secrets {set,get,rm,list}` against the secrets file.
- `codeless-server`, `codeless-client`, `codeless-tauri-desktop` —
  still stubs; Phase 2b+ work.

Verify the workspace any time with:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

All three are green as of the Phase 2a wrap-up commit. Real (non-mock)
runners (Claude, Anthropic, etc. from the vendored `ai-runner`),
worktree wiring inside `drive_job`, the HTTP/SSE server, and the
review/notifier surfaces are Phase 2b+ work.

## Durable project facts (update as the project evolves)

Add entries here when a fact becomes load-bearing and isn't already
captured in SCOPE.md. Keep entries short; if something needs more than
a paragraph, write a `DOCS/` page in the parent workspace and link to
it.

- **2026-05-12** — Bootstrap. Workspace created at
  `codeless-workspace`; `codeless` repo moved under it; vendored
  `ai-runner` from the rubix-agent workspace; mani.yaml + tasks
  written; CLAUDE.md established. Cargo workspace with 8 crate stubs
  landed on `feat/bootstrap-cargo-workspace`.
- **2026-05-12** — Phase 1 skeleton complete on
  `feat/bootstrap-cargo-workspace` (11 stages, 7 ticks, see
  `../DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md`). End-to-end
  `codeless run --once` works against the mock runner; secrets CLI
  and worktree manager both have integration coverage.
- **2026-05-12** — Phase 2a (persistence + queue) complete on
  `feat/phase-2a-persistence` (9 stages, 8 ticks, see
  `../DOCS/sessions/2026-05-12-phase-2a-persistence.md`). `MemoryStore`
  removed; repos/jobs/stages/tasks/events all live in SQLite via
  `SqliteStore`. Events allocate cursors from the autoincrement
  column; `subscribe(since)` does sqlx-backed replay then live
  broadcast tail with cursor-based dedupe at the boundary.
  Lease-based task queue with atomic three-scope concurrency caps
  (global / per-repo / per-runner), CAS completion/failure/heartbeat,
  `spawn_heartbeat` background helper, and startup-time lease reaper
  inside `with_db`. A resumability integration test
  (`tests/resumability.rs`) opens a file-backed SQLite, lands
  non-trivial state, drops the runtime, rebuilds against the same
  file, and proves repos/jobs/tasks/events all survive and the
  cursor allocator keeps climbing. Real-runner adoption from
  `ai-runner`, worktree threading inside `drive_job`, the HTTP/SSE
  server, and review/notifier surfaces are Phase 2b/2c work.
