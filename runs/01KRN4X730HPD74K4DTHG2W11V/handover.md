## Done

- F3c committed (17a74d2): planner now emits AssistantActionCard tool calls; cards persist as their own assistant-role rows with meta_json, surface through the new AppendAssistantMessageResult.cards field, and confirm_assistant_action drives the existing dispatcher end-to-end.
- AssistantThreadView appends res.cards so planner proposals render in the same round-trip.
- 6 new tests in codeless-runtime cover planner card emission, mixed text+card turns, unknown-tool rejection, and the full confirm path.
- Wire types regenerated (cargo run -p codeless-rpc --example wire_ts) and specta snapshot updated via SPECTA_UPDATE=1.

## Next

- Stage 7: F1 — drive AiInputBar from the current assistant thread so footer submissions land in /assistant's transcript on next render. Spec is in DOCS/ASSISTANT-SCOPE.md §F1 and DOCS/sessions/2026-05-15-assistant-followups.md stage 7.

## What you need to know

- AppendAssistantMessageResult gained `cards: Vec<AssistantMessage>` (serde default; TS `cards?: AssistantMessage[]`). Callers that want planner-emitted proposals must spread `res.cards` after `res.assistant_message`.
- Pure-card turns promote the first card into `assistant_message` (so old UIs still see something confirmable); `cards` then contains the trailing cards only. Mixed text+card turns put prose in `assistant_message` and all cards in `cards`.
- AssistantAction is `#[serde(tag = "tool")]`; parse_tool_call folds the runner's tool name back in as the discriminator. A smuggled `tool` key in args_json is rejected.
- Unknown tool names are logged with tracing::warn and dropped — surrounding prose still lands. If the policy needs to change to hard-fail, the spot is `parse_tool_call`'s Err arm in run_planner_turn.
- Pre-existing flaky tests on the base: `codeless-runtime::rpc_in_process::job_filtered_subscription_drops_unrelated_events` and `codeless-adapters-host::git_commit::tests::commit_paths_creates_commit_for_new_file` (cross-test cwd race). Confirmed not caused by F3c.
- Unrelated working-tree drift was left uncommitted (.codeless/jobs/fix-ai-agent/SCOPE.md, WORKFLOW.md, setup/ADDING-JOB.md, github_issue.rs, github_pr.rs, wire-rpc.ts.snap.actual, runs/01KRN4X730HPD74K4DTHG2W11V/). These predate this stage; deal with them separately.

## Open questions

- Should unknown tool names hard-fail the turn instead of being dropped? Current bias is permissive (drop + log) so a half-good reply still lands; revisit if telemetry shows the model hallucinating tool names regularly.
- Card-only turn promotes the first card to `assistant_message` for back-compat. Once F1's footer is wired and every consumer reads `cards`, consider always putting cards in `cards` and leaving `assistant_message` empty for pure-card turns.
