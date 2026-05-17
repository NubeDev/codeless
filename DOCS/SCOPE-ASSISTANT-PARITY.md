# SCOPE-ASSISTANT-PARITY

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-17

## Summary

Three concrete gaps between the `/assistant` surface and the rest of
the UI:

1. The assistant view does not subscribe to the event bus and renders
   message bodies as plain text. The planner already publishes
   `AiToken` / `ToolCall` envelopes to the bus
   ([`assistant_planner.rs:107`](../crates/codeless-runtime/src/rpc/assistant_planner.rs#L107))
   keyed on `JobId(thread_id.0)`. The UI just never wired up the
   subscription. `JobChat` ([`RunPane.tsx:1129`](../ui/codeless-ui/src/modules/jobs/RunPane.tsx#L1129))
   subscribes through `useEventStream` and renders through
   `Streamdown`; the assistant view has neither, so streaming reply
   tokens arrive as one block when the RPC returns and markdown is
   shown raw.
2. `CommonChat` ([`modules/chat/CommonChat.tsx`](../ui/codeless-ui/src/modules/chat/CommonChat.tsx))
   is a discriminated-union switch over three forked renderers
   (`JobChat`, `AiChatView`, `AssistantThreadView`). The "extraction"
   claimed in `ASSISTANT-SCOPE.md` M2 never happened; renaming the
   facade does not unify behaviour.
3. The planner has no awareness of `AutoBypassPolicy`. The seven
   variants live on `JobSpec` and `update_job` already; the
   `SubmitJobDialog` exposes them via `POLICY_PRESETS`
   ([`SubmitJobDialog.tsx:152`](../ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx#L152));
   the assistant cannot draft, change or recommend a policy and has
   no surface to set one when a job halts on a failure a non-`None`
   policy would have bypassed. The `draft_job` action card also has
   no shared form with `SubmitJobDialog`, so any field the planner
   leaves blank forces the user back to the `/jobs` tab.

This scope lands a real shared chat renderer, extracts the new-job
composer, and plumbs the policy through both. Acceptance: the same
DOM (modulo header) renders the message list for a job thread and an
assistant thread; the assistant's `draft_job` card is the
`SubmitJobDialog` form; the planner can recommend a policy in prose
and the user can confirm a change without leaving the chat.

## Out of scope

- Replacing or extending the planner itself (prompts, tool surface
  beyond the auto-bypass additions, model selection).
- Per-user permissions or any multi-tenant work.
- Attachments / image paste on the assistant view. `JobChat`'s
  attachment path is worktree-backed; assistant threads are
  workspace-scoped. `ASSISTANT-SCOPE.md` §Data model already records
  the decision (workspace-scoped dir); the *upload* surface is
  follow-up.
- Folding the in-editor `AiChat` into the assistant. That collapse is
  the next step after this scope; gating it on this scope keeps the
  change reviewable.

## Workstreams

### W1 — Real `CommonChat` renderer

The current facade stays as the public import; its body becomes the
shared renderer instead of a switch.

- Lift the message-list + streaming-text + tool-card chrome out of
  `JobChat` into `modules/chat/CommonChat.tsx`. Subscribe through
  `useEventStream` with a caller-supplied `EventFilter` (job threads
  pass `{ scope: "job", job_id }`; assistant threads pass
  `{ scope: "job", job_id: thread_id }` because the planner already
  publishes on that key). Render bubbles through `Streamdown`.
- Wrappers shrink: `JobChat` keeps its header, scope-edit chrome and
  `CHAT.md` history loader; `AssistantThreadView` keeps its title
  bar; `AiChatView` keeps its SDK-backed history loader (full
  collapse is deferred — see Out of scope).
- History loader is a prop: `JobChat` passes a `parseChatMarkdown`
  loader against `CHAT.md`; assistant passes `list_assistant_messages`;
  the renderer is loader-agnostic.
- Retire `focusStore.refreshTick` once the live channel is the
  source of truth for "rail re-sort" — the planner emits a thread-
  touched envelope the rail subscribes to.
- Tests: render the assistant thread with a fake event channel
  delivering `AiToken` deltas and assert the bubble accumulates
  identically to `JobChat`'s existing streaming-text test;
  parity test asserts the same message rows produce identical
  message-list DOM whether wrapped by `JobChat` or
  `AssistantThreadView`.

Touch points:

- `ui/codeless-ui/src/modules/chat/CommonChat.tsx` — becomes the
  real renderer (was: switch).
- `ui/codeless-ui/src/modules/jobs/RunPane.tsx` — `JobChat` shrinks
  to a wrapper; the streaming/tool-card chrome moves out.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  shrinks to a wrapper; polling path deleted.
- `ui/codeless-ui/src/modules/assistant/focusStore.ts` —
  `refreshTick` deleted once the rail subscribes to the channel.
- `ui/codeless-ui/src/modules/ai/components/AiChat.tsx` — opt-in
  to the renderer but keeps its SDK transport (full migration is
  follow-up).

No server change. The bus envelope already exists.

### W2 — Extract `JobComposer`

`SubmitJobDialog` splits into a thin shell that owns
open-state + submission, and a `JobComposer` component that owns the
field set.

- New: `ui/codeless-ui/src/modules/jobs/composer/JobComposer.tsx`.
  Props: `value: JobSpecDraft`, `onChange(next)`, `errors`,
  `repo`, `mode: "create" | "edit"`. No `RpcClient` writes — the
  shell does the `jobs.create` / `jobs.update` call.
- All validation (slug rules, branch-sync default, runner required,
  caps positive, custom-policy-requires-comment) lives in the
  composer and is exposed through `errors`.
- `SubmitJobDialog` becomes the shell; assistant's `draft_job` card
  mounts the same composer pre-populated from the planner's
  proposed `JobSpec`.
- Tests: `SubmitJobDialog`'s existing field-level tests move to
  `JobComposer`; the dialog keeps an open/submit shell test; the
  assistant card gets a round-trip test (planner draft → user edits
  → `jobs.create` receives the edited value).

No server change.

### W3 — Auto-bypass in the planner + cards

- The planner system prompt renders the variant list + one-line
  hints. Single source: `POLICY_PRESETS` in `SubmitJobDialog.tsx`
  is moved to `ui/codeless-ui/src/lib/policy/presets.ts` plus a
  matching `crates/codeless-runtime/src/auto_bypass/presets.rs`
  (or whichever module owns the variant list today). The planner
  prompt builder reads the Rust copy. UI and server stay aligned
  by both consuming the same enum + hints.
- `draft_job` card: embeds `JobComposer` (W2); the planner's draft
  carries an optional `auto_bypass_policy`; the picker defaults to
  `None` when unset.
- `update` card: when proposing a policy change mid-flight, the
  card honours the `AUTO-BYPASS-DECISIONS.md` Q5 paused-job rule.
  The dispatcher calls `set_job_policy`; the card surfaces the
  same "pause first" affordance the scope-edit-on-running-job
  card already uses.
- Failure-time card: when a job halts on a stage failure and the
  current policy is `None` (or the failure would have auto-bypassed
  under a non-`None` policy), the planner emits a one-shot
  `set_policy` action card offering a recommended variant + resume.
  Hidden on cap-breach halts (caps always halt, Q2). Informational
  only under `Relentless`.
- Tests: planner-prompt snapshot covers all seven variants;
  `update`-card dispatch test asserts the paused-job rejection
  surfaces typed; failure-card render test driven by a mocked
  stage-failure event.

Touch points:

- `crates/codeless-runtime/src/rpc/assistant_planner.rs` — prompt
  builder consumes the preset list.
- `crates/codeless-runtime/src/rpc/assistant.rs` — dispatcher gets
  a `set_policy` arm calling `set_job_policy`.
- `ui/codeless-ui/src/lib/policy/presets.ts` — extracted source.
- `ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx` — imports
  presets from the new path.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  renders the new card type (folds into W1's shared tool-card
  surface).

## Sequencing

1. **W2** first. Pure UI refactor, no dependencies, unblocks W3's
   draft-card work.
2. **W1** second. UI-only, parallelisable with W2. Blocks the W3
   failure-card / update-card surfacing because those cards belong
   in the shared tool-card chrome.
3. **W3** last. Consumes W2 (composer) and W1 (shared cards).

Each workstream is independently shippable behind the existing
route — partial completion is visible without feature flags.

## Acceptance

- The assistant thread streams tokens as they arrive and renders
  markdown through `Streamdown`.
- `SubmitJobDialog` and the assistant's `draft_job` card render the
  same form; a slug-validation change in `JobComposer` propagates
  to both with no copy-paste.
- The planner can answer "what's the auto-bypass policy on job X?"
  and "switch job X to long-term" through the same card surface
  that already handles `start` / `stop` / `pause`.
- When a job halts on a stage failure the assistant surfaces a
  `set_policy` card; confirming it pauses, sets the policy and
  resumes in one user action.
- `CommonChat` is a component, not a switch. Removing the
  `kind`-discriminated branches does not break callers.

## Non-acceptance / explicit non-goals

- AiChat does not collapse into the assistant in this scope; the
  shared renderer is opted-in by `AiChatView` but the SDK
  transport stays.
- Attachments on the assistant view are not added here.
- No changes to `AutoBypassPolicy` variants, semantics, or the
  paused-job rule. This scope plumbs an existing feature; it does
  not redesign it.
