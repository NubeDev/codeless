# Scope — scoped-pause-points

Pre-declared pause breakpoints in `template.yaml`. The operator writes
the plan and the pause points together; the runner halts itself at the
declared spots without anyone clicking pause at runtime. The existing
`pause_job` / `resume_job` / `JobStatus::Paused` machinery already
exists — this job adds the **scheduling layer** on top, not a new pause
primitive.

## Goal

After this job, `template.yaml` accepts a `pause_points:` list. Each
point names a stage (by ordinal or by name) and optionally a todo
within it (by ordinal or by `TodoKind`), plus a `position: before|after`.
At submit time the parser resolves symbolic names, validates against the
declared stages/todos, and rejects with a typed `ScopeError` before the
job leaves `draft`. At runtime, the stage/todo state machine consults
the schedule and triggers the existing `pause_job` path with
`StopReason::ScopedPausePoint { point_id }`. The Stage overview shows
a planned-pause divider at each scheduled point; the chat shows the
divider when the pause fires. `resume_job` advances past one point at a
time.

## In scope

- `pause_points:` grammar in `template.yaml`. Each entry is one of:
  - `{ stage: <ordinal | "name">, position: "before" | "after", reason?: "<str>" }`
  - `{ stage: <ordinal | "name">, todo: <ordinal | "checks" | "docs" | "git" | "<title-substring>">, position: "before" | "after", reason?: "<str>" }`
- Wire types in `codeless-types`: `PausePoint`, `PausePointPosition`,
  `PausePointTarget` (`Stage { ordinal }` | `StageTodo { stage_ordinal,
  todo_selector }`), `TodoSelector` (`Ordinal(u32)` | `Trio(TodoKind)` |
  `TitleSubstring(String)`), with `specta` derives and a `serde`
  round-trip test. iOS/Android-safe (no host deps).
- Template parser extension in `codeless-runtime`: resolve symbolic
  stage names to ordinals against the parsed `stages:` list at submit
  time; reject unknown stages, unknown trio kinds, out-of-range
  ordinals, and ambiguous title substrings with a typed `ScopeError`
  variant; refuse the submit (the job never reaches `draft` with a
  broken schedule).
- `scheduled_pause_points` SQLite table, one row per declared point,
  keyed on `(job_id, ordinal)` for stable ordering. Rebuilt
  idempotently inside `resync_template_from_disk` so a chat-driven
  template edit adjusts the schedule without orphan rows.
- Runtime hook in the stage/todo state-machine transitions: before
  advancing into a stage or a todo, the runtime checks the schedule;
  if a point matches, it calls the existing `pause_job` entry point
  with `StopReason::ScopedPausePoint { point_id }` and emits
  `JobPaused`. The closing-trio todos (`checks`, `docs`, `git`) are
  legal targets so "pause after stage 3 docs" works.
- New `StopReason` variant `ScopedPausePoint { point_id: PausePointId,
  label: String }`. The label is the human-rendered form ("before stage
  3 todo 1: implement parser") computed at trigger time so the chat
  divider does not have to re-resolve.
- UI: in the Stage overview, render a planned-pause divider chip at
  each scheduled point that has not yet fired. In the chat, render a
  "paused at <label>" divider when `JobPaused` carries the new
  `ScopedPausePoint` reason. The resume button is the existing
  `resume_job` surface — no new RPC. Playwright happy-path covers the
  divider rendering plus a resume click that advances to the next
  scheduled point.

## Out of scope

- Editing pause points from the UI. Operators edit `template.yaml`
  (directly or via the chat-driven on-disk edit path); the UI only
  renders the schedule and the dividers. Defer the edit-from-UI surface
  to a follow-up job.
- Conditional / predicate breakpoints ("pause if cost > $5",
  "pause on first regex match"). Out — declaratively-addressed points
  only.
- Recurring or count-based breakpoints ("pause every N stages",
  "pause on the third retry"). Defer.
- Soft vs hard pause semantics. This job uses whatever `pause_job`
  already does — no new pause primitive.
- Auto-resume / timed resume. Resume is always an explicit
  `resume_job` call.
- Cross-job synchronisation ("pause job A until job B completes").
  Single-tenant trust boundary (R5) but no inter-job choreography.

## Constraints

- **R1** — Schedule resolution and runtime hook live in
  `codeless-runtime`; wire types live in `codeless-types`. No new
  `tokio::process` / `std::process::Command` introduced anywhere; the
  feature touches state-machine code only.
- **R2** — The UI imports `RpcClient` only. The divider chip and chat
  marker read from `get_job` plus the existing event stream; no
  `@tauri-apps/*` import sneaks in.
- **R3** — One responsive component for the divider chip; no
  per-shell forks.
- **R4** — `scheduled_pause_points` is the source of truth. The UI
  does not cache the schedule; it re-reads on `template_resynced` and
  on the `pause_points_updated` event the resync emits.
- **R5** — No per-point auth. Same bearer gate as every other RPC.
- **R2 comments rule** — no task-status comments, no emojis, no
  restatements. Comments earn their keep only for *why*.
- `cargo test --workspace` / `cargo clippy --workspace --all-targets
  -- -D warnings` / `cargo fmt --check` all green before each commit.
- MSRV 1.78.

## Deliverables (what "done" looks like)

1. `codeless/scoped-pause-points` branch with one commit per stage,
   pushed via mani.
2. `cargo test --workspace` green. New tests:
   - Wire-type serde round-trip for `PausePoint` and its variants.
   - Parser rejection tests: unknown stage name, out-of-range ordinal,
     ambiguous title substring, unknown trio kind, conflicting
     duplicate point.
   - End-to-end: submit a job with three points, walk the
     state machine with the mock runner, assert the runner pauses
     three times and the `JobPaused` reason carries the right
     `point_id` + label each time.
3. `scheduled_pause_points` table migrates cleanly on a fresh DB and on
   the existing dev DB; resync test asserts that removing a point from
   `template.yaml` drops its row on the next resync.
4. UI Playwright: a job with one scheduled point renders the divider
   chip in the Stage overview, fires the chat divider when the pause
   hits, and the **Resume** click advances past the point.
5. `DOCS/SCOPED-PAUSE-POINTS.md` ships with the grammar, the rejection
   rules, and three worked examples (stage-only, stage+trio,
   stage+title-substring).

## Open questions — resolved in stage 1

The grammar, examples, and rejection rules these decisions encode live
in [`DOCS/SCOPED-PAUSE-POINTS.md`](../../../DOCS/SCOPED-PAUSE-POINTS.md).

1. **Default `position:` if omitted — `before` or required?**
   **Required.** Declaring a point without a position is a foot-gun:
   "pause stage 3" reads ambiguously to the next agent (halt before
   the runner spawns vs. halt after the closing trio). Forcing the
   keyword keeps the YAML self-explaining and removes one class of
   silently-wrong schedule. Parser rejects missing `position:` with
   `ScopeError::MissingOrInvalidPosition`.

2. **Title-substring selector — keep or drop?**
   **Keep.** It is the only way to address runner-authored
   intermediate todos that don't exist at submit time but show up
   after a `resync_template_from_disk`. The runtime cost is local —
   one case-insensitive `contains` per stage entry per selection
   tick — so the unhappy path doesn't tax the happy path. Ambiguity
   fails loudly: empty substring rejects at parse time
   (`EmptyTitleSubstring`), multi-match at bind time
   (`AmbiguousTitleSubstring`) halts the job into `Paused` with the
   error in `stop_reason_message` rather than choosing arbitrarily.

3. **Should `resync_template_from_disk` fire `JobPaused` retroactively
   if the user adds a point whose target stage already passed?**
   **No.** New points only apply to transitions that haven't happened
   yet. A point whose target has already run inserts with
   `superseded_at = <now>, fired_at = NULL` and the resync event
   payload lists the silenced point ids. Replaying pauses against
   already-completed state would lie about what the runner did and
   confuse the chat divider, which renders forward in time.

4. **`StopReason::ScopedPausePoint` — does it count toward the cost-cap
   reset semantics that other resumable pauses use?**
   **Yes.** A scoped pause is operator intent expressed ahead of
   time; from the runner's perspective it is indistinguishable from
   a manual `pause_job` followed by an immediate `resume_job`. The
   existing cap-reset path therefore applies unchanged — no new
   branch in the cap accounting, no new policy knob.

## Stage 1 deliverables (this stage)

- [x] `DOCS/SCOPED-PAUSE-POINTS.md` lands with §1 grammar, §2 three
  worked examples (stage-only, stage+trio, stage+title-substring),
  §3 rejection-rules table mapping each `ScopeError` variant to its
  trigger, §4 resync semantics, §5 the four open-question
  resolutions, and §6 a fully-resolved schedule worked example.
- [x] Four open questions resolved above with one-line *why* each.
- [ ] Stages 3-8 deliverables — pending later stages.

## References

- Existing pause/resume types: `crates/codeless-types/src/job.rs`
  (`JobStatus::Paused`, `StopReason`)
- Existing pause/resume events: `crates/codeless-types/src/event.rs`
  (`JobPaused`)
- Existing RPCs: `crates/codeless-rpc/src/methods.rs`
  (`pause_job`, `resume_job`, `resync_template_from_disk`)
- Stage / todo model: `crates/codeless-types/src/stage.rs`,
  `crates/codeless-types/src/todo.rs` (closing trio: `TodoKind::TRIO`)
- Workspace doc precedent: [`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
- Agent rules: [`CLAUDE.md`](../../../codeless/CLAUDE.md),
  [`../CLAUDE.md`](../../../CLAUDE.md)
