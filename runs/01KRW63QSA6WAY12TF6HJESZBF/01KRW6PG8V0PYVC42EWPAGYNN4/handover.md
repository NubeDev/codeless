## Done

- W2a: replaced the read-only `DraftJobPreview` inside pending `draft_job` action cards with the editable `JobComposer` the dialog shell uses; Confirm dispatches `submit_job` with the composer-edited args, locally flips the card to `confirmed`, and appends a synthetic `tool` row pointing at the new job.
- Committed only my targeted file + the new `.codeless/jobs/assistant-parity/{SCOPE.md,WORKFLOW.md,runtime.yaml}` (untracked job docs that this session referenced). Other pre-existing modified/untracked files in `codeless/` were left alone — they are unrelated WIP and should not be swept into this stage's commit.
- Branch `feat/assistant-parity`, commit `c7af399`, pushed.

## Next

- Stage W2b: round-trip test — planner-seeded `draft_job` card renders the composer with the planner's values, the user edits a field (e.g. cost cap), Confirm submits the edited value via `submit_job` (assert on a `MockRpcClient` mutation log). Add `AssistantThreadView.test.tsx`.
- REVIEW gate after W2b before W1 begins (per WORKFLOW.md).

## What you need to know

- Touch point file: `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx`. The new components `DraftJobComposerPanel` / `DraftJobComposerPanelInner` live in this file alongside the existing `DraftJobPreview` (which is still used for non-pending draft_job cards) — matches the file's established pattern of co-locating per-tool card renderers. `R3 (one concept per file)` justifies splitting if W3 grows these further.
- Planner emits no job-name field. The composer derives a name slug from the proposed branch by stripping the `codeless/` prefix and slugifying. The field is editable in the composer.
- Trade-off intentionally accepted: confirm goes through `submit_job` directly (not `confirm_assistant_action`), because the existing confirm path calls `draft_job_from_conversation` which reads the planner's original args off the card and ignores user edits. The card's persisted status stays `pending` on the server until a future RPC accepts edited args alongside the card id. The local UI flip prevents in-session double-submit. Comment in `onConfirmDraftJob` explains this.
- `JobComposer` reads `state.info` for the runner dropdown. The panel fetches `rpc.serverInfo()` once on mount (mirroring `SubmitJobDialog`) and `rpc.call("list_repos", {})` to resolve `action.repo_id` to a `Repo`. Both are fail-soft: a missing repo or info-fetch error surfaces a red banner inside the card.
- `pnpm test -- --run` (59/59 passing), `pnpm typecheck` clean. `pnpm lint` is a stub (no eslint configured). `cd ui/codeless-ui` first.
- Constraint greps: `grep -rn "@tauri-apps" ui/codeless-ui/src/modules/{chat,assistant,jobs}` and `grep -rn "\\.web\\.tsx|\\.desktop\\.tsx|\\.mobile\\.tsx" ui/codeless-ui/src` both empty — R2/R3 holding.

## Open questions

- SCOPE.md OQ#1 ("composer inline or popover in the draft_job card") — resolved inline per the parity-scope bias; the composer renders directly inside the card body with foldable prompt + scoped Confirm/Cancel. No popover.
- Persistence drift on the card status (server stays `pending` after submit). Either accept until §W3 lands an RPC for edited-args-confirm, or add a minimal `cancel_assistant_action` call after `submit_job` and reinterpret the "cancelled" status in this renderer. Left as documented trade-off for now; revisit in REVIEW after W2b.
