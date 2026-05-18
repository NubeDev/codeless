## Done

- Renamed `private jobs` to `private jobIndex` in `AssistantThreadView.setPolicy.test.tsx` to fix TS2416 introduced by ca196f4; typecheck is now clean
- Added a W3c-tagged round-trip test in `AssistantThreadView.draftJob.test.tsx` asserting the planner's `auto_bypass_policy: { type: "long-term" }` seeds the composer picker and round-trips onto `submit_job` (covers the "draft_job card embeds the policy picker" half of W3c)
- Committed and pushed as `0a8c337` on `feat/assistant-parity` via mani: 2 files changed, 54 +/4 -
- Full UI trio (`pnpm typecheck`, `pnpm test -- --run` = 15 files / 69 tests, lint = no-op script) green; R2/R3 greps clean

## Next

- W3d: failure-time `set_policy` card emitted on stage halt under `None` policy (runtime emits the card; AssistantThreadView already renders `set_policy` via the W3c `SetPolicyPanel`)
- After W3d: REVIEW before merge — end-to-end smoke of the SCOPE-ASSISTANT-PARITY.md Acceptance list

## What you need to know

- The W3c functional work (SetPolicyPanel + onConfirmPolicyAfterPause + policyLabel + wire-through ActionCardView, plus the initial setPolicy test) was already present in HEAD because commit `ca196f4` ("added email client") bundled it in alongside the email feature. This stage's commit (`0a8c337`) is the proper W3c-titled commit that lands the typecheck fix and the picker-seed test the parity contract requires
- `SetPolicyPanel` chooses its confirm path from the job status fetched via `get_job`: Running/AwaitingReview => "Pause & confirm" (pauses then dispatches), Draft/Stopped/Paused => standard Confirm via `confirm_assistant_action`, Queued/Completed/Failed => disabled with a typed reason. The dispatcher's `set_policy` arm in `crates/codeless-runtime/src/rpc/assistant.rs` already calls `set_job_policy`; the panel just honours the runtime's Q5 paused-job rule before dispatching
- `JobComposerInitial` already carries `policy: action.auto_bypass_policy ?? null` (line 617 of AssistantThreadView.tsx), so the picker embedding for `draft_job` is a no-op at the wiring level — the new draftJob test locks in that the mapping survives
- The `mock-client.ts` `set_job_policy` arm only rejects Running/AwaitingReview, but the actual runtime (jobs.rs:782-799) also rejects Queued and other non-{Draft,Stopped,Paused} statuses. The panel's UX follows the runtime's stricter rule, so tests against the mock still reflect production behaviour
- No session doc exists at `DOCS/sessions/2026-05-XX-assistant-parity.md`; previous stages have managed the job via `.codeless/jobs/assistant-parity/{SCOPE,WORKFLOW,REVIEWS}.md` and `runtime.yaml`. Continuing that convention

## Open questions

- (none)
