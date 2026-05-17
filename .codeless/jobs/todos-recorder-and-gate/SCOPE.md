# Scope — todos-recorder-and-gate

## Goal

Wire the Todo feature end-to-end on top of the foundation that
already shipped (types, three Event variants, `0021_todos.sql`,
`SqliteStore::insert_todo` / `update_todo_status` / `trio_resolved`,
unit tests). When this job lands:

- Every running task has a visible checklist in the `Stages` overview
  tab, driven by `todo-added` / `todo-updated` / `todo-completed`
  events.
- The closing trio (`checks`, `docs`, `git`) is **runtime-injected**
  at stage entry — the runner cannot skip it.
- `StageCompleted` is held back until `SqliteStore::trio_resolved`
  returns true for the stage's terminal task.
- The `Stages` tab shows the trio ticking over in real time so a
  30-minute Claude session no longer looks stalled.

The contract is in [`DOCS/SCOPE.md`](../../DOCS/SCOPE.md) under the
`Todo` row and the SSE event list at L818; the UI mock is in
[`DOCS/JOB-UI.md`](../../DOCS/JOB-UI.md) "Todo rows (nested under a
tick)" and "The mandatory closing trio".

## In scope

- StageRecorder handling of `TodoAdded` / `TodoUpdated` /
  `TodoCompleted` → store calls. Pattern matches the existing task
  event handling in `stage_recorder.rs`.
- Stage-entry trio injection: at the point that emits
  `StageStarted`, also enqueue three `TodoAdded` events
  (`Checks` / `Docs` / `Git`) with the three highest ordinals on
  the stage's first task. Use `TodoKind::TRIO` for ordering.
- Stage-completion gate in `state_machine.rs` / `template_runner.rs`:
  before emitting `StageCompleted`, call `trio_resolved` on the
  terminal task. If false, keep the stage open and log the missing
  rows.
- `verify_runner.rs` emits `TodoUpdated(checks, InProgress)` /
  `TodoCompleted(checks, Done|Failed)` around the verify run.
- The handover-writer step emits the `docs` trio updates.
- The per-stage commit+push step emits the `git` trio updates,
  using `Skipped` for the no-diff case (matches `update_todo_status`
  behaviour and the docstring in `todos.rs`).
- Runner integration: detect Claude Code's `TodoWrite` tool call in
  the existing `ai-runner` event path and translate it into
  `TodoAdded` / `TodoUpdated` events. Out-of-scope runners (Codex,
  Copilot, Anthropic REST) get a stub that returns no todos —
  acceptable for first cut; the trio still fires regardless.
- UI: `src/components/jobs/StagesTab.tsx` (or wherever the overview
  lives) gains a third nesting level under tick rows, driven by the
  three new events. Glyph mapping is in `JOB-UI.md` "Todo status
  glyph".

## Out of scope

- Planner-authored todos (`TodoKind::Planner`). Phase-later.
- Re-keying the SSE wire format — all three events are additive
  and already merged.
- Reworking the Stage-detail tab. Todo rows only appear in the
  `Stages` overview for this job.
- A "fan-out trio per task" model — the trio is per-stage,
  attached to the stage's terminal (final) task, not to every task.
- Persisting todo titles longer than 200 chars. Truncate at the
  emit site if needed; the schema has no length cap on purpose
  but the UI does.

## Constraints

- R1 (process spawn only in `codeless-adapters-host`) still
  applies. Trio injection and the gate live in `codeless-runtime`,
  no new host-only deps.
- R4 (SQLite is source of truth). Trio resolution reads through
  `trio_resolved`, never an in-memory cache.
- R2 (UI imports `RpcClient` only). The Stages tab subscribes
  through `RpcClient.subscribe()`, no direct fetch / Tauri imports.
- The runtime injects the trio. A misbehaving runner that never
  emits a trio row is a runtime bug to surface, not a silent skip.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo fmt --check` all green before any commit
  lands.

## Open questions

1. Where in the lifecycle does the trio get injected — at
   `StageStarted` (current best guess) or at the point the stage's
   final task is enqueued? The latter avoids the "which task owns
   the trio" question but pushes the visible checklist later.
2. When the gate holds a stage open, does the UI see a new
   `StageWaiting` event or just the absence of `StageCompleted`?
   Stage 4 (REVIEW) is the right place to decide; defaulting to "no
   new event, the UI infers" preserves wire stability.
3. For runners other than Claude Code, do we emit a single synthetic
   `runner` todo per task ("running …") so the row isn't blank, or
   leave the runner level empty and only show the trio? Defaulting
   to "leave it empty" — the trio alone covers the "did it land?"
   question, which is what the user actually asked for.
