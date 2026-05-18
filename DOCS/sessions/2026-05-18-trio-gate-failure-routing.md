# Trio gate failure routing + heartbeat

Branch:       `fix/trio-gate-failure-routing` (inner `codeless/`)
Status file:  this file
Related:     [`2026-05-18-trio-gate-wiring-fix.md`](./2026-05-18-trio-gate-wiring-fix.md)
              — the earlier session that wired the trio resolvers but
              left the gate semantics unchanged.

## Symptom

Job `01KRX4ZPF10J3QZ35R5GK8336X` ran stage 2 tick 1 for ~110 minutes
under `auto_bypass_policy={"type":"long-term"}` before the operator
stopped it manually. SQLite forensics:

- `jobs.status = stopped`, `stop_reason = user` — the user clicked
  Stop, the runtime did not exit on its own.
- `stages.status = running` (never closed), `ended_at = NULL`.
- One terminal task on the stage: `tasks.status = running`,
  `ended_at = NULL`.
- Three trio todos on that task:
  - `checks` → `skipped`
  - `docs`  → `failed`  (handover write failed)
  - `git`   → `skipped`
- Event stream after the docs-rail flip stops at cursor 5539
  (`todo-completed{status:skipped}`); the next event is the
  `job-stopped` at cursor 5553, ~88 minutes later.

The trio rows were terminal in 400 ms. The stage row stayed `running`
for the remaining 88 minutes because **the closing-trio gate spun in a
100 ms poll loop with no way out**.

## Root cause

[`crates/codeless-runtime/src/store/todos.rs`](../../crates/codeless-runtime/src/store/todos.rs)
defined `trio_resolved(task_id) -> bool` where `true` meant "all three
rows in `Done` or `Skipped`". The `Failed` terminal status returned
`false`, and there was **no path that could ever flip it back** — the
emitters (`claude_runner` for `docs`, `verify_runner` for `checks`,
`trio_emitter::commit_stage_changes` for `git`) write a single
terminal status per stage and never retry.

[`crates/codeless-runtime/src/template_runner.rs::wait_for_trio_resolved`](../../crates/codeless-runtime/src/template_runner.rs)
polled that boolean every 100 ms, indefinitely, with one `tracing::info!`
on first iteration and then silence. The `auto_bypass_policy` machinery
exists for stage failures — but the gate never produced one. It just
waited.

## Fix (this branch)

Four wedged-together changes; together they make a failed trio row a
*stage failure*, not a *poll-forever condition*.

### 1. Tri-state gate result

[`store/todos.rs`](../../crates/codeless-runtime/src/store/todos.rs)

```rust
pub enum TrioGateOutcome {
    Resolved,
    Pending,
    Failed { failures: Vec<TrioFailure> },
}
```

`trio_gate_outcome(task_id)` replaces `trio_resolved(task_id)`. A row
ending `Failed` produces `TrioGateOutcome::Failed { failures }`
immediately; `Failed` *wins over* `Pending` because a terminally-failed
peer cannot un-fail, so polling further is pointless.

`TrioFailure { kind, reason }` carries the per-rail reason picked up
from `todos.failure_detail` (new column, migration `0023`).

### 2. Persisted per-rail failure reason

- Migration `0023_todo_failure_detail.sql` adds
  `todos.failure_detail TEXT` (nullable).
- `Todo` struct (`codeless-types`) gains
  `failure_detail: Option<String>`.
- `Event::TodoCompleted` gains
  `failure_detail: Option<String>` (additive wire field; `#[serde(default,
  skip_serializing_if = "Option::is_none")]` keeps the wire backward-
  compatible).
- `update_todo_status` now takes `Option<&str>` and writes the column.
- All three emitters thread a real string in:
  - `claude_runner.rs`: `format!("write handover: {err}")`
  - `verify_runner.rs`: `format!("verify step {step_index} exited {exit_code}")`
  - `trio_emitter::commit_stage_changes`: `format!("git commit failed: {err}")`

### 3. Gate timeout + heartbeat

[`template_runner.rs::wait_for_trio_resolved`](../../crates/codeless-runtime/src/template_runner.rs)
returns `TrioGateWaitOutcome { Resolved | Failed{reason} | TimedOut{reason} | Cancelled }`.

- `TRIO_GATE_MAX_WAIT = 300s` — if the gate cannot resolve in 5 min
  something is wedged; surface as a stage failure.
- `TRIO_GATE_HEARTBEAT = 10s` — every 10 s while `Pending`, emit
  `Event::StageTrioGateWaiting { stage_id, waiting_on: Vec<TodoKind>,
  elapsed_ms }` so the UI can show "waiting on: docs, git" instead of
  dead silence.
- New wire variant `stage-trio-gate-waiting` added to `Event` (additive,
  Specta-regen.).

### 4. Failure routed through existing auto-bypass

The gate's `Failed` and `TimedOut` outcomes both route through the
**same** path as any other stage failure: `StageCompleted { Failed,
FailureClass::RunnerError, failure_detail: <reason> }` →
`classify_stage_failure` → `try_auto_bypass`. Jobs with
`auto_bypass_policy = long-term` (or any other policy) now advance past
a failed trio rail; jobs without a policy halt with a real reason on
the stage row.

### 5. UI surface

[`ui/codeless-ui/src/modules/jobs/StagesOverview.tsx`](../../ui/codeless-ui/src/modules/jobs/StagesOverview.tsx)

- `TodoRow.failureDetail?: string | null` plumbed through reducer.
- `TodoChildRow` renders the rail's `failure_detail` inline under the
  row in muted-mono, error-toned, so an operator sees *which* rail
  failed and *why* without diving into server logs.

## Tests added / updated

- `store/todos.rs`:
  - `update_status_persists_failure_detail_on_failed` — column write.
  - `trio_gate_outcome_pending_until_all_three_kinds_resolve` — replaces
    `trio_resolved_requires_all_three_kinds`.
  - `trio_gate_outcome_failed_when_any_trio_row_failed` — replaces the
    bug-encoding `trio_resolved_false_when_any_trio_row_failed`. This
    is the direct regression test for job `01KRX4ZPF...`.
  - `trio_gate_outcome_failed_wins_over_pending` — pins the policy that
    a terminal failure short-circuits the gate even when peers are
    still in-progress.
- `state_machine.rs::trio_gate_blocks_until_all_three_resolved` —
  updated to the tri-state.
- `template_runner.rs::failed_trio_row_routes_stage_to_failed_with_detail`
  — full end-to-end through the mock runner + recorder + bus: assert
  that flipping the docs row to `Failed` with a reason emits
  `StageCompleted { Failed, failure_detail: "...docs...disk full..." }`.

All `cargo test --workspace`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo fmt --check` pass. UI typecheck + 23/23 test
files / 118 tests pass.

## What this does NOT change

- The trio's runtime injection (`state_machine.rs`) is unchanged.
- The auto-bypass policy code path (`auto_bypass_guard.rs`,
  `auto_bypass_failure_card.rs`, `try_auto_bypass`) is unchanged — the
  gate failure flows through the existing seam.
- Runner-emitted (non-trio) todos are unaffected; `failure_detail` is
  optional everywhere.
- No retry logic was added. A failed trio rail is a failed stage.

## How to verify against the original failure

```sh
# Recreate the situation: docs rail fails, others pass.
cargo test -p codeless-runtime --lib failed_trio_row_routes_stage_to_failed_with_detail
```

The test runs the full template_runner + recorder + bus pipeline, fails
the docs row, and asserts the gate publishes `StageCompleted { Failed
}` with the rail's reason in `failure_detail`. Before this fix the same
test would have hung in the wait loop until the test harness timed out.

## Operational notes

- Migration `0023` is additive (nullable column). Existing job rows are
  unaffected; existing todo rows keep `failure_detail = NULL`.
- The wire-format addition (`TodoCompleted.failure_detail`,
  `StageTrioGateWaiting`) is `serde(default)` so older clients keep
  decoding new events.
- An old server emitting `TodoCompleted` without `failure_detail` to a
  new client decodes as `None` — same on-disk behaviour as a successful
  rail. The UI's failure-detail surface only renders when status is
  `failed`, so the worst case is "stage failed with no reason" instead
  of "stage hung forever" — strictly better.
