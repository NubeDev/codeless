## Done

- Wrote `ui/codeless-ui/src/modules/assistant/AssistantThreadView.draftJob.test.tsx`: vitest integration test that mounts `AssistantThreadView` against a stubbed `MockRpcClient` subclass seeded with a single pending `draft_job` action card, edits the cost-cap field from $5 -> $10, clicks Confirm, and asserts `submit_job` was called with `cost_cap_cents: 1000` (user edit, not the planner's 500), `repo_id`, `runner: "claude"`, `branch: "codeless/parity-w2b"`, `start_immediately: false`. Also asserts the card flips out of pending (no more Confirm button) and that a `Drafted job` synthetic tool row is appended.
- Verified guards: `pnpm test` (12 files / 60 tests pass), `pnpm typecheck` (clean), `pnpm lint` (no eslint configured), `grep -rn "@tauri-apps"` and shell-fork grep clean.
- Committed as `df0faa9 W2b round-trip test for planner-seeded draft through user edits to submit_job` on `feat/assistant-parity`. Not pushed (no instruction in this tick to push).

## Next

- Stage 3 of 13 is `REVIEW before W1 — composer parity verified before the renderer rewrite begins`. The review-gate criteria are in `.codeless/jobs/assistant-parity/WORKFLOW.md#review-gates` item 1: confirm planner emits no `draft_job` field the composer cannot accept (the W2a commit already maps `runner / branch / cost_cap_cents / wall_clock_cap_ms / workspace_mode / model / permission_mode / effort / auto_bypass_policy` — every field on the wire), and that the composer emits no submit_job field a regression would silently lose.
- After REVIEW passes, Block 2 (W1a) starts on `CommonChat`.

## What you need to know

- Stage commits live on branch `feat/assistant-parity`. W2a (c7af399) added the editable composer panel inside the assistant `draft_job` card; W2b (df0faa9) is the round-trip test for that wiring. No session doc under `DOCS/sessions/` was created at W2a; the per-job worklist files in `.codeless/jobs/assistant-parity/` are the live session surface.
- `MockRpcClient` predates the assistant RPCs and throws `unhandled method` for `list_assistant_messages` / `confirm_assistant_action` / `cancel_assistant_action` / `append_assistant_message`. The new test subclasses it with `AssistantStubMock` to canned-respond for `list_assistant_messages` and to record `submit_job` args; this is the same pattern used by `JobDetailStack.parallel.test.tsx` with its `RecordingMock`. If future stages need more assistant RPCs in tests, extend the subclass rather than reaching into the base.
- The worktree had unrelated dirty state on entry (modifications to `crates/codeless-adapters-host/src/lib.rs`, `crates/codeless-cli/src/serve.rs`, `ui/codeless-ui/src/lib/rpc/mock-client.ts`, several others, plus untracked `crates/codeless-adapters-host/src/net.rs` and `DOCS/SETTINGS-INTEGRATIONS.md`). I committed only my new file via direct `git add <path>` to avoid sweeping the drift into a "W2b" commit. `mani run commit` would have used `git add -A` and conflated them; do not use mani here until the drift is resolved or confirmed unrelated.

## Open questions

- The dirty workspace state above is unattributed — looks like leftover work from a previous job. Before pushing this branch, confirm with the human whether those files should be part of the assistant-parity branch or reverted.
- WORKFLOW.md asks for a `DOCS/sessions/2026-05-XX-assistant-parity.md` session doc created at W2a; W2a didn't create one. Not retroactively created here. The REVIEW stage may want to backfill it.
