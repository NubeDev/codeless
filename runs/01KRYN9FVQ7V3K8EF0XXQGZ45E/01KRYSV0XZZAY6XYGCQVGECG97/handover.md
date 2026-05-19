## Done

- Verified `JobTimeline.tsx` chip tooltip already carries `failure_class` + `failure_detail` via `comment_used` (stage-7 threading from `auto_bypass_policy::policy_comment`); no source change required.
- Added `ui/codeless-ui/src/modules/jobs/JobTimeline.test.tsx` with two vitests: (a) bypassed-after-fail renders the chip + tooltip containing the wire-name class and verbatim detail, (b) hard-fail-no-bypass produces zero auto-bypass chips on `verify-failed` + `stage-completed{failed}`.
- `npx vitest run` green on JobTimeline.test.tsx (2/2), and the existing StagesOverview + ReviewGatePanel tests stay green (23/23). `tsc --noEmit` clean.
- Committed as `246a779` on `codeless/auto-bypass-hardening` with message starting `stage 11:` (matching the existing stage-N misalignment in this branch's history — stage 11 commit slot is stage 13 of the job per the prior commits).

## Next

- Stage 14 of 15 (per the job goal): pick up the next item in the auto-bypass-hardening sequence. Branch is `codeless/auto-bypass-hardening`; commit is pushed locally only (no `git push` ran — mani is not available inside this worktree).

## What you need to know

- CLAUDE.md requires commits via `./bin/mani --config mani.yaml run commit --projects codeless …` from the workspace root, but `bin/mani` is not present in this worktree, so the commit went through raw `git` with the conventional message instead. Re-running through mani from the outer workspace if/when available will not change file content.
- The wire event `StageAutoBypassed` carries only `{stage_id, policy_name, comment_used, applied_at}` — no separate `failure_class`/`failure_detail` fields. The threading is purely inside `comment_used` (the runtime composes it at emit time via `policy_comment(policy, Some(PriorFailure { class, detail }))`). Tests assert against the substring shape `Previous-stage failure: <class>\nDetail: <detail>` to pin that contract.
- Overview-side coverage (`StagesOverview.test.tsx`) and gate-panel coverage (`ReviewGatePanel.test.tsx`) for the bypass story were already in place from stages 10–12; the new file fills the timeline gap only.

## Open questions

- (none)
