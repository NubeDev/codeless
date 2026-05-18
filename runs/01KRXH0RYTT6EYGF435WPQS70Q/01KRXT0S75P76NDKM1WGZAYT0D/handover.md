## Done

- Added migration `crates/codeless-runtime/migrations/0026_supervisor_goals.sql` with columns `id`, `run_id`, `kind`, `condition_json`, `action_json`, `authorised_by` (FK → `chat_messages.id`), `status`, `created_at`, `fired_at`, plus partial index `idx_supervisor_goals_armed` on `(run_id, created_at) WHERE status='armed'` for cheap supervisor-boot rehydration.
- Added `crates/codeless-runtime/src/store/supervisor_goals.rs` with typed enums (`SupervisorGoalKind`, `GoalCondition`, `GoalAction`, `ThresholdMetric`, `GoalStatus`, `SupervisorGoalId`, `SupervisorGoal`, `MarkOutcome`, `GoalValidationError`, `InsertGoalError`, `ExecutionState`) and `SqliteStore` methods `insert_goal`, `list_armed_for_run`, `mark_fired`, `mark_cancelled`, `mark_superseded`. Validation re-serialises typed payloads through serde and refuses kind/condition mismatches at write time; mark_* helpers are idempotent (terminal-already → `NoChange`).
- v0.1 kinds = `deadline-stop` / `threshold-stop` / `event-notify`; `pause_after_stage` is parsed as a `GoalAction::PauseAfterStage` variant and surfaces `ExecutionState::NoOpFailed` per "no-op until JOB-WORKFLOW (A.5)".
- Wired module + re-exports in `crates/codeless-runtime/src/store/mod.rs`.
- 6 new unit tests cover insert/list round-trip, kind/condition mismatch decode error, mark_fired idempotency + audit-trail preservation, cancelled/superseded terminal no-op, pause_after_stage NoOpFailed signal, and armed-only scan ordering. `cargo test -p codeless-runtime --lib`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all green. Committed as `d672403` ("stage 13: supervisor_goals migration + store").

## Next

- Stage 14 (supervisor reactor wiring): consume `list_armed_for_run` on supervisor boot to re-arm timers / bus subscriptions; recognise "if X then Y" chat turns and call `insert_goal`; route fired goals through `Tools::stop_job` / `post_chat_message` and `mark_fired`; route `PauseAfterStage` through the `NoOpFailed` audit path until JOB-WORKFLOW (A.5) lands.

## What you need to know

- `run_id` is typed as `JobId` in `SupervisorGoal` because the JOB-WORKFLOW (B) `RunId` newtype has not landed; SQL column stays `TEXT` so the swap is value-only (same pattern as `chat_messages.run_id`).
- `condition_json` / `action_json` are validated by `serde_json::to_string`/`from_str` against the tagged enums; `kind` column is derived from the condition variant in `insert_goal` so callers cannot pass it separately.
- `authorised_by` is a FK on `chat_messages(id)`. Tests seed a real chat row first; future supervisor wiring should ensure the user turn is persisted before inserting the goal.
- `cargo test --workspace` had one flaky failure in `codeless-adapters-host::git_commit::tests::commit_paths_is_noop_when_nothing_changed` (parallel-CWD git race) — passes on re-run, unrelated to this stage.
- Commit was made with raw `git` since this worktree has no mani access (`../bin/mani` and `../mani.yaml` absent); the parent JOB-LOOP harness should push via mani when it picks up the branch.

## Open questions

- (none)
