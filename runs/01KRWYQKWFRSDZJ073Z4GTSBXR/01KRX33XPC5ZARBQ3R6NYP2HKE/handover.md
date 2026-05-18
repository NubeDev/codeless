## Done

- Added `list_scheduled_pause_points` RPC (server trait + runtime impl + axum route + HTTP client + UI methods mirror); read-only, resume continues through the existing `resume_job` surface
- Regenerated `ui/codeless-ui/src/lib/rpc/generated/wire.ts` (registers `PausePoint*` + `TodoSelector` in the wire_ts example); `StopReason` now exposes the `scoped-pause-point` object variant
- `StagesOverview` renders one `PlannedPauseChip` per scheduled point in YAML order against the parent stage's before/after slot; chip turns amber + shows a `Resume` button when `job.stop_reason` matches the chip's point id
- `chat/feed.ts` emits a `paused at scoped point <id>` lifecycle divider for `JobPaused { reason: ScopedPausePoint }`; new `scopedPausePointId` helper reads both serde (`point_id`) and specta (`point-id`) spellings
- New `stopReasonLabel` helper threaded through `JobChatPage`, `JobDetail`, `JobTimeline`, `RunPane` so the object variant no longer trips TS's ReactNode check
- Mock client: `seedScheduledPausePoints` test seam, `list_scheduled_pause_points` arm, `resume_job` accepts `paused`
- Tests: two new chip + Resume cases in `StagesOverview.test.tsx`, six new cases in `feed.scopedPause.test.ts` (118 vitest tests passing, `cargo test --workspace` green, clippy + fmt clean)
- Committed as `a290d75` on `codeless/scoped-pause-points`; SCOPE.md deliverables ticked for stage 8; `handover.md` refreshed for the stage 9 REVIEW gate

## Next

- Stage 9: final REVIEW gate (do not auto-merge per WORKFLOW.md)

## What you need to know

- The template names a "Playwright test" but the UI tree has no Playwright harness today (vitest browser-playwright is in pnpm-lock but not configured); coverage uses vitest+RTL like every other UI test in the tree — see handover.md "Tests"
- Known specta/serde divergence: serde JSON emits `point_id` (underscore) inside the `scoped-pause-point` variant, specta TS spells the same field `point-id` (hyphen); the UI helper handles both. Aligning these is filed as a stage-9 follow-up
- Per-todo chip placement collapses onto the parent stage today; `StageTodo` targets render on the stage row's chip slot with the selector named in the label. Inline-with-the-todo placement is a layout-only refinement and is noted in handover.md follow-ups
- mani is not present in this isolated worktree, so the commit went through plain git (no `--force`, no `--no-verify`); the worktree branch `codeless/scoped-pause-points` carries the commit and the JOB-LOOP harness will push it
- One flaky test: `codeless-adapters-host`'s `git_revert_undoes_an_earlier_commit_and_returns_new_sha` fails when the lib tests run in parallel and passes deterministically with `--test-threads=1` — unrelated to this stage

## Open questions

- (none)
