# Workflow — capture Claude session id

How the agent drives the three stages. They are independent
add-only changes; a failure in stage 2 still leaves stage 1's types
on the branch.

## Stage 1 — types + event variant

Add `pub session_id: Option<String>` to `Stage` in
`crates/codeless-types/src/stage.rs`. Add a new event variant
`Event::StageSessionCaptured { stage_id: StageId, session_id:
String }` in `crates/codeless-types/src/event.rs`. Plumb the
field through `StageRollup` in `crates/codeless-rpc/src/methods.rs`.

Regenerate the TS wire snapshot:

```sh
cargo run -p codeless-rpc --example wire_ts
```

Copy the regenerated `wire.ts` into
`ui/codeless-ui/src/lib/rpc/generated/wire.ts` (the existing build
step does this; do not hand-edit).

Verify by running `cargo test --workspace`. No behavioural tests
yet — this stage only changes the wire shape. The drift test on the
TS snapshot must be green.

Commit as `capture-session-id stage 1: add Stage.session_id + StageSessionCaptured event`.

## Stage 2 — runtime capture + persistence

Add the `session_id TEXT NULL` column to the `stages` table via an
additive migration in `crates/codeless-runtime/src/store.rs`. Update
encode/decode to round-trip the new field.

In `crates/codeless-runtime/src/stage_recorder.rs`, when a task
reports a `RunResult` with a non-empty `session_id`:

1. If the stage row's `session_id` is already `Some(_)`, do nothing
   (idempotent — multi-task stages keep the first capture).
2. Otherwise: write the session id to the DB, update the in-memory
   stage row, and publish `Event::StageSessionCaptured { stage_id,
   session_id }` to the bus.

Add a unit test in `stage_recorder` that drives a fake task with a
known session id and asserts:
- the stage row in SQLite has `session_id = Some("sess-fake")`,
- exactly one `StageSessionCaptured` event was emitted,
- a second task on the same stage with a *different* session id
  does **not** re-emit the event and does **not** overwrite the
  stored value.

Verify by `cargo test --workspace`. The new unit test must pass;
no existing test breaks.

Commit as `capture-session-id stage 2: persist session_id in StageRecorder`.

## Stage 3 — UI consumer

In `ui/codeless-ui/src/modules/jobs/StageDetail.tsx`, replace the
"Claude session ID" placeholder card body with the actual value
when `stage.session_id` is non-null. When null, keep the existing
placeholder copy so mock-runner jobs (which never emit a session
id) still look right.

Verify by:

1. `pnpm -C ui/codeless-ui tsc --noEmit` — green.
2. `pnpm -C ui/codeless-ui build` — green.
3. `make start` (or the equivalent `cargo run -p codeless-cli --
   --db ... serve --enable-claude`), submit a real Claude-runner
   job, open StageDetail for any completed stage — the card shows
   `sess-<ulid>` rather than the placeholder.

Manual verification only at the UI layer is fine; there is no
component test harness wired for StageDetail yet, and writing one
is out of scope for this job.

Commit as `capture-session-id stage 3: render Stage.session_id in StageDetail`.

## What counts as done

All three stages committed cleanly on the job branch. `cargo test
--workspace`, clippy, tsc, and pnpm build all green. A Claude-runner
job submitted via the UI shows a real session id in StageDetail.
The `stages` table in SQLite has the new column and the migration
runs cleanly on an existing dev DB (`.codeless-dev/codeless.db`)
without data loss.

## What to avoid

- No drive-by refactors of `StageRecorder` beyond the capture
  logic. Per `CLAUDE.md` R4.
- No status comments ("added in stage 2", "for the session-id
  task"). Per `CLAUDE.md` R2.
- Do not push the branch. The user reviews the worktree before
  any push.
- Do not migrate existing rows to backfill `session_id`. New rows
  only — old rows remain `None`. Backfill from event history is a
  separate, deferrable concern.
- Do not change `RunResult.session_id` (it is upstream
  `ai-runner` shape). This job only consumes what's already there.
