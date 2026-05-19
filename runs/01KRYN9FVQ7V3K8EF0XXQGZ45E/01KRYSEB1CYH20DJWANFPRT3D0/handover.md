## Done

- Added the failed+bypassed glyph (`~`, muted-foreground, "bypassed after failure" aria-label) to `stageGlyph` in `ui/codeless-ui/src/modules/jobs/StagesOverview.tsx`; flag is ignored on non-failed rows
- Threaded `bypassedPolicy` + `failureDetail` onto `StageData`, captured from `stage-auto-bypassed` (`policy_name`) and `stage-completed` (`failure_detail`); `mergeRollup` seeds both from `list_stages` with `bypassed_reason` as the policy fallback
- Wired tooltip `auto-bypassed by <policy>: <failure_detail>` (with detail-missing fallback) onto the stage title span, plus `data-bypassed="true"` for test reach
- Extended `ui/codeless-ui/src/lib/rpc/methods.ts` `StageRollup.stage` with optional `bypassed_at`/`bypassed_reason`/`failure_class`/`failure_detail` so strict-mode TS accepts the rollup read
- Added three vitest cases (`StagesOverview render — bypassed-after-failure`) pinning glyph mapping, rendered tooltip + aria-label, and the policy-only fallback
- `vitest run src/modules/jobs/StagesOverview.test.tsx` → 16/16 pass; `tsc --noEmit` clean
- Committed as `7050920` on `codeless/auto-bypass-hardening`

## Next

- (none) — stage 13 is the next session's job

## What you need to know

- Commit went via raw `git` (worktree has no `bin/mani`); workspace-root mani push is the operator's job after worktree merge
- `stageGlyph` is now exported so the test can pin glyph mapping without going through the full render path
- `bypassedPolicy` prefers the event-sourced `policy_name`; rollup-only cold opens fall back to the rollup's free-text `bypassed_reason`, which may not be one of the five preset labels — the tooltip just renders whatever string is there
- No snapshot files exist (vitest has no `__snapshots__` dir in this repo); the "vitest snapshot updated" phrase in the stage brief was satisfied by the new render-assertion tests

## Open questions

- (none)
