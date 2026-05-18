# Scope — assistant-parity

## Goal

The /assistant surface today is missing three things the rest of the
chat UI already has, all enumerated in
[`DOCS/SCOPE-ASSISTANT-PARITY.md`](../../DOCS/SCOPE-ASSISTANT-PARITY.md).
That doc is the source of truth for this job; this file is the
per-stage worklist that turns its three workstreams (W1, W2, W3)
into reviewable ticks.

End state — quoted from the parity doc Acceptance section:

- The assistant thread streams tokens as they arrive and renders
  markdown through `Streamdown` (today: home-grown `MarkdownBubble`,
  no `Streamdown`).
- `SubmitJobDialog` and the assistant's `draft_job` card render the
  same form (today: dialog uses `JobComposer`, the assistant card
  uses a read-only `DraftJobPreview` `dl` table — see
  [`AssistantThreadView.tsx:442-476`](../../ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx#L442-L476)).
- The planner can recommend and apply an `AutoBypassPolicy` through
  the same card surface that already handles `start` / `stop` /
  `pause` (today: planner has no awareness of the seven variants;
  `POLICY_PRESETS` lives in the composer module only).
- `CommonChat` is a component, not a switch (today: a 51-line
  discriminated-union facade over three forked renderers — see
  [`CommonChat.tsx`](../../ui/codeless-ui/src/modules/chat/CommonChat.tsx)).

## In scope

This job ships **all three workstreams** of
[`SCOPE-ASSISTANT-PARITY.md`](../../DOCS/SCOPE-ASSISTANT-PARITY.md).
The order is W2 → W1 → W3 because:

- **W2 first** is one targeted edit that immediately closes the
  "nice way to create jobs from chat" gap. The composer is already
  extracted (`useJobComposerState`, `composerToSubmitArgs`,
  `JobComposer`); only the wire-up to the assistant card is
  missing.
- **W1 second** is the larger refactor — lifting streaming +
  Streamdown + tool-card chrome out of `JobChat` into `CommonChat`.
  Doing W2 first means the assistant card already mounts shared
  components, so W1's renderer landing inherits a wired card path
  rather than having to retrofit one.
- **W3 last** because the failure-time + update cards live inside
  the shared tool-card chrome W1 builds. Landing W3 before W1
  would mean writing card UX twice.

### W2 — wire JobComposer into the assistant draft_job card

Touch points:

- [`ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx`](../../ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx)
  — replace `DraftJobPreview` (the read-only table at lines
  442–476) with an editable `<JobComposer state={...} />`. The
  Confirm button calls `composerToSubmitArgs(state)` → `submit_job`.
- The composer's `useJobComposerState` already accepts an `initial`
  prop (see
  [`JobComposer.tsx:48-66`](../../ui/codeless-ui/src/modules/jobs/composer/JobComposer.tsx#L48-L66))
  — map the planner's `AssistantAction<{ tool: "draft_job" }>`
  shape onto `JobComposerInitial`. The planner already emits
  `runner`, `branch`, `cost_cap_cents`, `wall_clock_cap_ms`,
  optional `workspace_mode` / `model` / `permission_mode` /
  `effort` — every field the composer needs.
- The composer's `hideRunImmediately` prop is already there for
  exactly this case ([`JobComposer.tsx:349`](../../ui/codeless-ui/src/modules/jobs/composer/JobComposer.tsx#L349)).
  Pass it.

Tests:

- New unit test in `AssistantThreadView.test.tsx`: a planner-seeded
  `draft_job` card renders the composer with the planner's values,
  the user edits the cost cap, Confirm submits the edited value via
  `submit_job` (assert on a `MockRpcClient` mutation log).

### W1 — real CommonChat renderer

Touch points exactly as enumerated in the parity doc §W1 "Touch
points":

- `ui/codeless-ui/src/modules/chat/CommonChat.tsx` — becomes the
  real renderer.
- `ui/codeless-ui/src/modules/jobs/RunPane.tsx` — `JobChat` shrinks
  to a wrapper.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  shrinks to a wrapper.
- `ui/codeless-ui/src/modules/assistant/focusStore.ts` —
  `refreshTick` deleted once the rail subscribes to the channel.
- `ui/codeless-ui/src/modules/ai/components/AiChat.tsx` — opt-in
  to the renderer but keeps its SDK transport (full collapse is
  follow-up; parity doc §Non-acceptance).

Tests: parity test asserts the same message rows produce identical
message-list DOM whether wrapped by `JobChat` or
`AssistantThreadView`. Streaming test asserts the assistant bubble
accumulates `AiToken` deltas identically to `JobChat`.

### W3 — auto-bypass-aware planner + cards

Touch points exactly as enumerated in the parity doc §W3 "Touch
points":

- `crates/codeless-runtime/src/rpc/assistant_planner.rs` — prompt
  builder consumes the preset list.
- `crates/codeless-runtime/src/rpc/assistant.rs` — dispatcher gets
  a `set_policy` arm calling `set_job_policy`.
- `ui/codeless-ui/src/lib/policy/presets.ts` — extracted source
  for `POLICY_PRESETS`.
- `crates/codeless-runtime/src/auto_bypass/presets.rs` (or
  whichever module owns the variant list today) — matching Rust
  source the planner prompt reads.
- `ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx` and the
  composer module — import presets from the new path.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  renders the new card types (folds into W1's shared tool-card
  surface).

Tests: planner-prompt snapshot covers all seven variants;
`update`-card dispatch test asserts the paused-job rejection
surfaces typed; failure-card render test driven by a mocked
stage-failure event.

## Out of scope

Exactly the parity doc's "Non-acceptance / explicit non-goals":

- Folding `AiChat` into the assistant (the in-editor panel's
  collapse is a follow-up job, gated on this one).
- Attachments / image paste on the assistant view (workspace-scoped
  upload surface is follow-up).
- Replacing or extending the planner itself beyond the auto-bypass
  additions.
- Per-user permissions, multi-tenant work, OIDC.

## Constraints

- **R2** (`UI imports RpcClient only`): every new surface must
  route through `RpcClient`. No `@tauri-apps/*` imports anywhere
  in the touched files.
- **R3** (`one UI framework, forever`): no `Foo.web.tsx` or
  `Foo.desktop.tsx`. The renderer is one file.
- **R4** (`SQLite is the source of truth`): the streaming buffer
  is presentation state only; persisted messages stay in
  `assistant_messages` / `CHAT.md` / the AI SDK store as today.
  The renderer is loader-agnostic per the parity doc §W1.
- **R5**: bearer token continues to authorise every call; no
  per-thread auth scopes are introduced.

## Sequencing dependency on the live `todos-recorder-and-gate` job

[`todos-recorder-and-gate`](../todos-recorder-and-gate/SCOPE.md)
is running in parallel on branch
`codeless/todos-recorder-and-gate`. Its stage 6 touches
`modules/jobs/StagesOverview.tsx` (or wherever the stages tab
lives) to render todo rows nested under ticks. **No file overlap**
with this job:

- `assistant-parity` touches `modules/assistant/*`,
  `modules/chat/CommonChat.tsx`, `modules/jobs/RunPane.tsx`
  (`JobChat` wrapper only), and (W3) `modules/jobs/composer/*`
  + `modules/jobs/SubmitJobDialog.tsx`.
- `todos-recorder-and-gate` touches the stages-overview tree, the
  runtime stage recorder + state machine, and the ai-runner
  event path.

The two jobs share `modules/jobs/` as a directory ancestor but
not as a file. Merge order at PR time is FCFS; both branches
should rebase clean on master.

## Open questions

1. **Should the embedded composer in the assistant card render
   inline or in a popover?** Bias: inline. The card's whole point
   is the user reviews fields in context; pushing them into a
   modal undoes that. Resolve at stage W2a.
2. **Does W1 need to preserve the assistant view's polling-style
   `refreshTick`?** Parity doc says no — the planner's
   thread-touched envelope replaces it. Confirm the envelope is
   already published or add it at stage W1c. Resolve at W1c.
3. **W3 presets — share via a generated wire file or
   hand-mirrored?** Bias: hand-mirror with a CI assert. Seven
   variants do not justify the generator. Resolve at W3a.
