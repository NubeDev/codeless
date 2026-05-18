# scoped-pause-points — stage 1 → stage 2 (REVIEW gate)

Stage 1 was design-only. No code committed. Stage 2 is the REVIEW
gate; do not start stage 3 until the gate is approved in chat.

## What landed

- `DOCS/SCOPED-PAUSE-POINTS.md` (new) — the load-bearing grammar
  reference for the parser that lands in stage 4. Sections:
  - §1 Grammar — `pause_points:` key, `stage` (ordinal or name),
    `todo` (ordinal | trio kind | `~substring`), `position` (required,
    `before`|`after`), `reason` (optional, 512-byte cap).
  - §2 Three worked examples — stage-only, stage+trio, stage+title-
    substring with deferred resolution.
  - §3 Rejection-rules table mapping every `ScopeError` variant to
    its trigger. Parser pass order (structural → stage → todo →
    cross-point) pinned for tests in stage 4.
  - §4 Source-of-truth and re-resolution — schedule rows are written
    keyed on `(job_id, ordinal)`; `resync_template_from_disk` diffs
    in place; past-target points land `superseded_at` rather than
    firing retroactively.
  - §5 Open-question resolutions (mirror of job SCOPE.md §"Open
    questions — resolved in stage 1").
  - §6 Fully resolved schedule walk-through against a 9-stage plan.

- `.codeless/jobs/scoped-pause-points/SCOPE.md` — four open questions
  resolved with one-line *why* each:
  1. `position:` required (no default — ambiguous otherwise).
  2. Title-substring kept; ambiguity fails loud (parse-time empty,
     runtime multi-match).
  3. Resync does **not** retroactively fire `JobPaused`; past-target
     points are silenced no-ops with a one-line note in the resync
     event payload.
  4. `StopReason::ScopedPausePoint` resets cost caps the same way as
     manual `pause_job` — operator intent is operator intent.
  Stage 1 deliverables marked `[x]`.

- `DOCS/SCOPE.md` — one bullet added under the Appendix A "Notes"
  block cross-referencing `SCOPED-PAUSE-POINTS.md` and naming the
  one new `StopReason` variant (`ScopedPausePoint`) and the one new
  table (`scheduled_pause_points`, keyed on `(job_id, ordinal)`).

## What the stage 2 reviewer should check

1. Each of the four open questions in the job SCOPE has a single
   decisive answer plus one-line rationale. No hedging.
2. `DOCS/SCOPED-PAUSE-POINTS.md` §3 covers every `ScopeError`
   variant the parser in stage 4 will emit — adding a variant later
   means going back to this doc, not silently coding past it.
3. The worked example in §6 resolves cleanly against the schedule
   row layout described in §4 (the `ordinal` is the YAML index, not
   a re-numbered position after name resolution).
4. Cross-link sanity: job SCOPE.md ↔ `DOCS/SCOPED-PAUSE-POINTS.md`;
   `DOCS/SCOPE.md` → `DOCS/SCOPED-PAUSE-POINTS.md`.

## What stage 3 (next code stage) needs from this

Wire types in `codeless-types`:

- `PausePoint { id, target, position, reason }`
- `PausePointPosition { Before, After }`
- `PausePointTarget { Stage { ordinal }, StageTodo { stage_ordinal, selector } }`
- `TodoSelector { Ordinal(u32), Trio(TodoKind), TitleSubstring(String) }`
- `PausePointId` newtype.

All derive `serde`, `specta::Type`, `Debug`, `Clone`, `PartialEq`,
`Eq`. Serde round-trip test required. No host deps (iOS / Android
safe per R1).

The `StopReason::ScopedPausePoint { point_id, label }` variant lands
later (stage 6, with the runtime hook). Stage 3 does **not** touch
`StopReason` — the wire types alone can land without changing the
runtime.

## Open follow-ups for later stages (do not act on these now)

- Stage 4 parser: emit the pass-1 errors as a `Vec<ScopeError>` so
  the operator sees all structural issues at once.
- Stage 5 persistence: confirm `scheduled_pause_points` migration
  works against an existing dev DB with in-flight jobs; rebuild on
  `resync_template_from_disk` must be idempotent.
- Stage 6 runtime hook: place the schedule check at the same point
  where the trio-gate inspects stage completion, so `after stage N`
  semantics align with the existing `StageCompleted` emission.

No code commits in this stage; `cargo test --workspace`, clippy,
and fmt were not re-run (no Rust files changed).
