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

Phase 1 of SCOPE.md has landed on `feat/bootstrap-cargo-workspace`:

- `codeless-types` — Repo/Job/Stage/Task/Event/Review structs, serde +
  specta. iOS/Android-safe.
- `codeless-rpc` — `RpcServer` trait, args, results, error variants,
  subscribe surface. iOS/Android-safe.
- `codeless-runtime` — `InProcessRpc` over `MemoryStore` +
  `EventBus`, job/stage/task transition guards, `Runner` trait +
  `MockRunner` scripted harness, `drive_job` driver, tracing
  subscriber (`try_init_json` / `try_init_pretty`), sqlx migrator for
  the Appendix A schema. Host-only.
- `codeless-adapters-host` — `SecretStore` (chmod 0600, atomic-rename
  TOML), `WorktreeManager` (`git worktree add/remove/prune`).
  Host-only; the only crate permitted to spawn processes.
- `codeless-cli` — `codeless run --repo <p> "<prompt>"` end-to-end
  against the mock runner, streaming JSON-line events; `codeless
  secrets {set,get,rm,list}` against the secrets file.
- `codeless-server`, `codeless-client`, `codeless-tauri-desktop` —
  still stubs; Phase 3+ work.

Verify the workspace any time with:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

All three are green as of the Phase 1 wrap-up commit. Real (non-mock)
runners, worktree wiring inside `drive_job`, the SQLite-backed event
log, and the HTTP/SSE server land in Phase 2+.

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
  and worktree manager both have integration coverage. Real runner
  adoption (`ClaudeRunner` etc. from `ai-runner`), the SQLite-backed
  event log, and the HTTP/SSE server are Phase 2 work.
