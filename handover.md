# scoped-pause-points — stage 6 → stage 7 (REVIEW: server-side complete)

Stage 6 (runtime hook) landed. Stage 7 is the mid-job REVIEW gate; do
not start stage 8 (UI) until approved. The handover is the input the
reviewer reads when deciding whether to approve.

## What landed in stage 6

- `crates/codeless-runtime/src/scoped_pause_hook.rs` — new module.
  Exports `TransitionPoint` (one variant per hook site: `BeforeStage`,
  `AfterStage`, `BeforeTodo`, `AfterTodo`), `check_and_pause` (the
  per-transition entry point), and `check_trio` (the convenience
  forwarder the trio_emitter call sites use; it resolves the
  `StageId → stage_ordinal` lookup so the trio sites stay one
  function call thick). The matcher is total per `(target, position,
  transition)` triple and the `selector_matches` helper covers all
  three `TodoSelector` variants — `Trio { kind }`, `Ordinal { ordinal }`,
  `TitleSubstring { pattern }` (case-insensitive `contains`,
  trio-rows refused by spec).

- `crates/codeless-types/src/job.rs` — new `StopReason::ScopedPausePoint
  { point_id: PausePointId }` variant. `StopReason` keeps `Copy`
  because `PausePointId` wraps a `Ulid` which is `Copy`; the variant
  carries the id only — the human-readable label is reconstructed
  downstream by stage 8 looking up the `scheduled_pause_points` row
  on render. This is a small deviation from the §6 worked example
  in SCOPED-PAUSE-POINTS.md ("StopReason::ScopedPausePoint
  { point_id, label }"): keeping `Copy` avoids touching ~40
  `StopReason`-Copying call sites; the label rides on the
  point-row's `reason` column instead of the wire, which is the
  same data path the divider chip uses.

- `crates/codeless-runtime/src/store/codec.rs` — `stop_reason_label`
  now returns `String` (was `&'static str`) so the scoped variant
  can encode as `scoped-pause-point:<ulid>` in SQLite's
  `jobs.stop_reason` column. `parse_stop_reason` splits on the
  colon prefix and reconstructs the variant. The six existing unit
  variants keep their bare-word labels — old rows decode unchanged.

- `crates/codeless-bot-core/src/notify.rs` and `reply.rs` — both
  `stop_reason_word` helpers grew a `ScopedPausePoint { .. }` arm so
  the Telegram / Slack / chat reply paths render "scoped pause
  point" instead of bombing the exhaustive match.

- Runtime hook call sites in `crates/codeless-runtime/src/template_runner.rs`:
  - **BeforeStage:** at the top of the per-stage loop body (line ~584,
    immediately after the cancel check + ordinal-already-passed skip,
    before `StageId::new()`). Maps `stage.index` (0-based) →
    `stage_ordinal: u32 = stage.index + 1` to match the YAML
    1-based ordinals the parser writes.
  - **AfterStage:** after `prev_stage_id = Some(stage_id)` at the
    bottom of the per-stage loop body, so the closing trio's
    `StageCompleted{Passed}` is already on the wire by the time the
    point fires. On `Paused`, returns `RunnerOutcome::Failed { reason:
    "scoped pause point" }`; the driver's existing
    `current.status == JobStatus::Paused` early-return short-circuits
    the Failed translation, so the row stays in `Paused`.

- Runtime hook call sites in `crates/codeless-runtime/src/trio_emitter.rs`:
  - **BeforeTodo:** at the top of `emit_trio_started`, before
    `find_trio_id`. Fires only for the trio kinds (`Checks`, `Docs`,
    `Git`) — the runner-authored todo path doesn't go through this
    function, so substring/ordinal targets land in the broader
    `BeforeTodo` hook the future stage 6.5 follow-up will wire into
    `claude_runner` once we decide where in the agent loop is the
    right cut point.
  - **AfterTodo:** at the bottom of `emit_trio_completed`, after the
    `TodoCompleted` publish so the row is durable before the pause
    divider lands. Same trio-only scope as `BeforeTodo`.

- `crates/codeless-runtime/tests/scoped_pause_hook.rs` — new
  integration test file. Three tests:
  1. `before_stage_hook_pauses_then_resume_requeues` walks the full
     pause-then-resume cycle against `InProcessRpc`: seeds a Running
     job with a `BeforeStage(2)` schedule, calls `check_and_pause`,
     asserts the row moved to `Paused` with
     `stop_reason = ScopedPausePoint { point_id }`, asserts
     `JobPaused` with the right reason landed on the bus, then
     `resume_job`s the row and confirms `Queued + stop_reason = None`.
  2. `hook_is_idempotent_against_already_paused_row` — calling the
     hook against a row that's already in `Paused` returns
     `HookOutcome::Continue` (the `transition_job` guard rejects
     `Paused → Paused`).
  3. `non_matching_transition_does_not_pause` — a transition that
     doesn't match any scheduled point leaves the row in `Running`.

- Plus six unit tests inside `scoped_pause_hook.rs` for the matcher
  itself: `BeforeStage`, `AfterStage`, trio-kind (matches kind +
  stage, refuses Runner kind), title-substring (matches Runner only,
  case-insensitive, scoped to stage), and ordinal (exact match).

## Verify

- `cargo test --workspace` — green. Three new tests in
  `crates/codeless-runtime/tests/scoped_pause_hook.rs` + six unit
  tests in the new module.
- `cargo clippy --workspace --all-targets -- -D warnings` — green.
- `cargo fmt --check` — green (one cosmetic rewrap on
  `pause_point.rs` from rustfmt's preference for inline struct
  variants; harmless).
- Specta snapshots updated (`SPECTA_UPDATE=1` in two spots): the
  `StopReason` TS union grew the `ScopedPausePoint` object variant
  and `PausePointId` now appears in `wire-rpc.ts.snap`. The
  field-name kebab-casing specta does on the inner struct (`point-id`
  in TS vs `point_id` in serde JSON) is the known specta-serde
  divergence the existing types accept; stage 8 will consume it via
  the typed RpcClient anyway.

## Runtime hook placement — the one-paragraph note the workflow asks for

Four call sites total. `BeforeStage` lives in the per-stage loop body
in `template_runner.rs` immediately after the cancel/prior-pass
guards but before any per-stage row is allocated, so a pause writes
the job state before SQLite picks up any stage frame for the halted
ordinal. `AfterStage` lives at the bottom of the same loop body
after `StageCompleted{Passed}` lands, so the closing trio is visible
in the timeline before the divider chip appears. The two todo-level
hooks live inside `trio_emitter::emit_trio_started` /
`emit_trio_completed`, which is the single seam every trio rail
(`verify_runner` for `Checks`, the claude `Docs` writer, the commit
step for `Git`) already flows through — wiring there avoids touching
three callers and keeps the hook surface one match call wide. The
hook is a no-op when no `pause_points:` rows exist; the SQL cost is
one `SELECT ... ORDER BY ordinal` per transition (bounded by the
declared schedule size, which is operator-authored and small).

## Open follow-ups (out of stage 6 scope; UI stage and beyond)

- The runner-authored todo path (the agent's `TodoWrite`-equivalent
  tool calls in `claude_runner`) doesn't run the `BeforeTodo` /
  `AfterTodo` hook yet, so a `~migrate` substring target only fires
  against trio rows (which never have a runner-authored title anyway,
  so the practical impact is zero today). Wire it into the agent
  loop once stage 8 lands and we have a UI surface that exercises
  the substring path end-to-end.
- The `fired_at` / `superseded_at` columns from SCOPED-PAUSE-POINTS §4
  are still deferred — the current hook re-evaluates the full schedule
  on every transition and the matcher's "first row wins" rule is
  enough to keep a single point from firing twice in one stage. Add
  the columns when the UI surfaces "this point already fired" state.
- A `pause_points_updated` event variant on `Event` for the divider
  chips to refresh without re-reading the whole job state. Same
  carryover from stage 5.

## What stage 7 (REVIEW) needs to assess

- New wire types: `StopReason::ScopedPausePoint { point_id }`.
- New `StopReason` variant exposes through the codec round-trip path
  as `scoped-pause-point:<ulid>` (see `parse_stop_reason` in
  `crates/codeless-runtime/src/store/codec.rs`).
- The four call sites are search-targetable as
  `crate::scoped_pause_hook::check_and_pause` and
  `crate::scoped_pause_hook::check_trio` in the runtime tree.
- The hook never bypasses `transition_job` — a non-Running row is a
  no-op (`HookOutcome::Continue`) rather than a forced flip.
