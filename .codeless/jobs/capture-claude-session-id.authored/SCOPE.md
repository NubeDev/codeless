# Scope — capture Claude session id on the Stage row

A small, mechanical, well-scoped change that pulls the Claude session
id out of `RunResult.session_id` (already in flight today) and pins
it onto the `Stage` row so it survives session boundaries and is
queryable from the UI without inspecting raw events.

This is the first of the [`PROGRESS.md` U2 wishlist
items](../../../DOCS/PROGRESS.md#u2--stagedetail-wishlist-m-parallelisable-with-a1)
and a useful first test of the codeless-developing-codeless loop:
three independent stages, deterministic verify, no soft judgement
calls.

## What success looks like

After all three stages run cleanly:

1. `Stage` (wire type) gains an `Option<String>` `session_id` field.
2. `StageRollup` exposes the same field.
3. A new `Event::StageSessionCaptured { stage_id, session_id }`
   variant exists. The runtime's `StageRecorder` emits it the first
   time a task on that stage reports a non-empty `session_id`,
   updates the `stages.session_id` column in SQLite, and never
   emits it twice for the same stage.
4. `list_stages` returns rows with `session_id: Some(...)` for any
   stage that ran against a real (non-mock) runner.
5. `StageDetail.tsx` renders the captured session id in the
   "Claude session ID" placeholder card. Placeholder copy stays
   only when the value is `None`.

Each stage commits its own files to the job's branch with a
descriptive message. No file outside the listed touchpoints
changes.

## Wire shape

```rust
// codeless-types/src/stage.rs
pub struct Stage {
    // ...existing fields...
    pub session_id: Option<String>,
}

// codeless-types/src/event.rs — new variant
Event::StageSessionCaptured {
    stage_id: StageId,
    session_id: String,
}
```

`session_id` is a free-form string. Claude's session ids look like
`sess-<ulid>`; other runners may use a different shape. Validation
is the runner's job, not this layer — we just persist whatever
`RunResult.session_id` carries.

## Touchpoints (expected — agent may discover more)

- `crates/codeless-types/src/stage.rs` — add field.
- `crates/codeless-types/src/event.rs` — new variant.
- `crates/codeless-rpc/src/methods.rs` — `StageRollup` exposes field.
- `crates/codeless-rpc/examples/wire_ts.rs` (or wherever specta
  emits) — regenerate TS snapshot.
- `crates/codeless-runtime/src/stage_recorder.rs` — capture on first
  task with a non-empty `session_id`; emit event; update DB.
- `crates/codeless-runtime/src/store.rs` — migration adding a
  `session_id TEXT NULL` column to `stages`, plus encode/decode.
- `ui/codeless-ui/src/lib/rpc/generated/wire.ts` — mirror the
  regenerated snapshot.
- `ui/codeless-ui/src/modules/jobs/StageDetail.tsx` — render the
  field.

## Out of scope

- Per-stage commits (U2 wishlist #2). Separate job.
- Tool-call ribbon (U2 wishlist #3). Separate job.
- Final assistant message excerpt (U2 wishlist #4). Separate job.
- Any A1 (handover) work. This is preparation; not handover itself.
- Re-using the captured session id for any kind of resume. Per
  [SCOPE.md hard rule #1](../../../DOCS/SCOPE.md#hard-rules-for-the-coding-runner),
  no `resume_id` / `--continue` across sessions. We capture for
  *observability*, not for resume.

## Constraints

- MSRV stays 1.78; no new dependencies.
- `cargo test --workspace` green at the end of each stage.
- `cargo clippy --workspace --all-targets -- -D warnings` green.
- `pnpm -C ui/codeless-ui tsc --noEmit` green at end of stage 3.
- Migration is additive; old `stages` rows read back with
  `session_id: None`. No data loss.
- All work happens on the job's worktree branch; never on `master`.

## Why this task as the first test

- Smallest A1-shaped change: same crates A1 will touch
  (`codeless-types`, `codeless-runtime`, `codeless-rpc`) plus a UI
  consumer, so it exercises the same surface area.
- Verify is binary: the field is `Some(_)` or it isn't.
- No new judgement-call code paths — the data is already flowing,
  we're just pinning it.
- Three obviously-independent stages: types → runtime → UI. Each
  passes its own tests in isolation.
