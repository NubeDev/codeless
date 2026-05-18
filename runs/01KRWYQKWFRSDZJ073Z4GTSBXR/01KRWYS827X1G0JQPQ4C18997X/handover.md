## Done

- Created `DOCS/SCOPED-PAUSE-POINTS.md` with grammar (`pause_points:` key, stage/todo/position/reason fields), three worked examples (stage-only, stage+trio, stage+title-substring), full `ScopeError` rejection-rules table, parser pass order, resync semantics, and a fully-resolved schedule walk-through.
- Resolved the four open questions in `.codeless/jobs/scoped-pause-points/SCOPE.md`: `position:` required; title-substring kept with loud ambiguity (parse-time empty, runtime multi-match); resync does not retroactively fire `JobPaused`; `StopReason::ScopedPausePoint` resets cost caps the same way as manual `pause_job`.
- Added a cross-reference bullet in `DOCS/SCOPE.md` Appendix A naming the new `StopReason::ScopedPausePoint` variant and the new `scheduled_pause_points` table.
- Updated `handover.md` for stage 2 (REVIEW gate).
- Committed as `stage 1: design pause_points schema + rejection rules` on `codeless/scoped-pause-points`.

## Next

- Stage 2 is the REVIEW gate; do not start stage 3 until approved in chat.
- Stage 3 (next code stage) adds `PausePoint`, `PausePointPosition`, `PausePointTarget`, `TodoSelector`, `PausePointId` to `codeless-types` with serde + specta derives and a round-trip test. iOS/Android-safe; no host deps. Does **not** touch `StopReason` yet (that lands in stage 6 with the runtime hook).

## What you need to know

- Stage 1 is design-only per the workflow doc — no Rust files touched, so cargo test/clippy/fmt were not re-run.
- The job runs in an isolated git worktree; commits went via plain `git` (mani.yaml lives in the parent workspace, not in this worktree). The branch is `codeless/scoped-pause-points`.
- The schedule row `ordinal` is the YAML index of the entry, not a re-numbered position after stage-name resolution. Stage 5's persistence work depends on that being stable.
- The label format for `StopReason::ScopedPausePoint` is fixed in §6 of the design doc: `"<position> stage <ordinal>[ todo <selector>]: <reason>"` (trailing `: reason` omitted when absent).
- Title-substring selectors are bound at runtime, not at parse time — that is the only way runner-authored todos can be targeted, and is the reason ambiguity rejects late rather than early.

## Open questions

- (none) — all four open questions from the job SCOPE are resolved in this stage. Stage 2's reviewer either signs off on the design or sends it back; no new questions were surfaced during this stage.
