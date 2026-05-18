# Reviews — assistant-parity

Layer-1 gate verdicts for the three REVIEW stages in this job. Each
entry records the commit range under review, which invariants were
checked, and the sentinel line emitted on the handover.

## REVIEW before W1 — composer parity verified before the renderer rewrite begins

Commit range: `cc76d14..df0faa9` (W2a `c7af399`, W2b `df0faa9`).

Files touched:

- `.codeless/jobs/assistant-parity/{SCOPE,WORKFLOW}.md`,
  `.codeless/jobs/assistant-parity/runtime.yaml` — job docs only.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  embedded `JobComposer` in the editable branch of the `draft_job`
  action card; added `onConfirmDraftJob` that calls `submit_job` via
  `useRpc()`.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.draftJob.test.tsx` —
  new round-trip test using `MockRpcClient`.

Layer-1 invariants:

- R1 (crate dependency direction): no Rust files touched; no
  `std::process` / `tokio::process` introduced anywhere. Holds.
- R2 (single transport): added imports are React, `@/lib/rpc`,
  shadcn primitives, `@/lib/utils`, `@/lib/route`, `../chat`, and
  `../jobs/composer`. No `@tauri-apps/*` import added. Every server
  call goes through `rpc.call(...)` or `rpc.serverInfo()`. Holds.
- R4 (SQLite source of truth): the post-confirm status flip and
  synthetic tool row are presentation-only optimistic state and the
  code comment flags the server-card-still-pending follow-up to be
  addressed in W3. No client-resident chat store is added. Holds.
- R5 (single trust boundary): no per-job / per-thread auth scopes
  introduced; no multi-tenant code paths. Holds.
- Wire formats: no Rust types, no RPC methods, no migrations changed.
  The composer mapping consumes existing `SubmitJobArgs` and
  `AssistantAction { tool: "draft_job" }` shapes. Holds.

Verdict: PASS. The composer round-trip is wired through the existing
`RpcClient` boundary without introducing host-only dependencies or
new wire shapes, clearing the way for the W1 renderer lift.

## REVIEW before W3 — shared renderer and composer must be stable before policy cards land

Commit range: `df0faa9..660126b` (W1a `c0691cf`, W1b `0ee0305`, W1c
`0822b07`, W1d `660126b`). Unrelated commits inside the range
(`972b34c`, `9b0c397`, `48aec3a`) are from the parallel
`todos-recorder-and-gate` job and are not part of this review.

Files touched by the W1 stages:

- `ui/codeless-ui/src/modules/chat/{ChatBubble,ChatMessageList,
  LifecycleDivider,PulseDot,ToolCallCard,feed,format,index}.{ts,tsx}`
  and `ChatMessageList.test.tsx`, `CommonChat.parity.test.tsx` — the
  lifted renderer + parity coverage.
- `ui/codeless-ui/src/modules/jobs/RunPane.tsx` — `JobChat` shrinks
  to a wrapper around `ChatMessageList`.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx` —
  shrinks to a wrapper; subscribes per-thread to the new envelope.
- `ui/codeless-ui/src/modules/assistant/{AssistantFooterBar,
  AssistantPage,focusStore}.{ts,tsx}` — rail + footer subscribe to
  `AssistantThreadTouched`; `refreshTick` / `bumpRefresh` retired.
- `crates/codeless-types/src/event.rs`,
  `crates/codeless-types/tests/wire.ts.snap`,
  `ui/codeless-ui/src/lib/rpc/generated/wire.ts` — additive
  `Event::AssistantThreadTouched { thread_id }` variant + regenerated
  TS mirror.
- `crates/codeless-runtime/src/rpc/assistant.rs` — `publish_thread_
  touched` helper invoked at every `touch_assistant_thread` callsite;
  unit test asserts the envelope appears on the bus per turn.
- `.codeless/jobs/assistant-parity/SCOPE.md` — open question #2
  resolution recorded.

Layer-1 invariants:

- R1 (crate dependency direction): no `std::process` /
  `tokio::process` introduced. The only Rust crate touched is
  `codeless-runtime` (host-only); the helper publishes through
  `rpc.bus`, no new spawn paths. Holds.
- R2 (single transport): grep of `@tauri-apps` under
  `ui/codeless-ui/src/modules/{chat,assistant,jobs}` returns zero.
  New imports route through `@/lib/rpc` (`useEventStream`,
  `EventEnvelope`, `EventFilter`, `MockRpcClient`, `RpcProvider`)
  or are pure UI primitives (`motion`, `streamdown`, shadcn).
  Holds.
- R3 (one UI framework): grep of `.web.tsx|.desktop.tsx|.mobile.tsx`
  under `ui/codeless-ui/src` returns zero. The new `modules/chat/*`
  files are one concept per file (R3 within R3). Holds.
- R4 (SQLite source of truth): `touch_assistant_thread` SQL writes
  still precede every `publish_thread_touched` call; publish errors
  are downgraded to `tracing::warn` with an inline rationale that
  the envelope is a UI freshness hint, not the source of truth.
  Holds.
- R5 (single trust boundary): bearer token unchanged; no per-thread
  or per-job auth scopes added. Holds.
- Wire formats: the sole change is the additive
  `Event::AssistantThreadTouched` variant (matching `wire.ts.snap`
  row). No RPC method signatures, table schemas, or migration files
  touched. Backwards-compatible enum addition. Holds.

Verdict: PASS. Shared `CommonChat` renderer, wrapper-shrunk
`JobChat` / `AssistantThreadView`, and the `AssistantThreadTouched`
envelope replacing `refreshTick` all land without violating Layer-1
or breaking wire compatibility. The renderer and composer surface
W3 will mount its policy cards into is stable.

PASS: shared renderer + composer land within R1/R2/R3/R4/R5 and the
only wire change is the additive `AssistantThreadTouched` envelope
backing the `refreshTick` retirement.

## REVIEW before merge — end-to-end smoke against SCOPE-ASSISTANT-PARITY.md Acceptance

Commit range: `9d53514..13866ec` (W3a `4a0f83e`, W3b `16bbdf6`,
W3c `0a8c337`, W3d `13866ec`). Unrelated commit `ca196f4`
("added email client") bundled `crates/codeless-tools/src/email/*`
and `tools/gmail_send.rs` into the range; those files do not touch
the assistant / chat / composer surface and are out of scope for
the parity acceptance list but are noted here for transparency.

Files touched by the W3 stages (excluding the unrelated email
commit):

- `crates/codeless-runtime/src/auto_bypass_presets.rs` (new) — Rust
  mirror of the seven `POLICY_PRESETS` keyed by the existing
  `AutoBypassPolicy` enum.
- `crates/codeless-runtime/src/auto_bypass_failure_card.rs` (new) —
  best-effort `set_policy` action card emitted on `JobFailed` when
  `auto_bypass_policy` is `None` and `stop_reason` is not a cap.
- `crates/codeless-runtime/src/{driver,job_driver_loop,lib}.rs` —
  wire the failure-card emitter into the terminal-state path.
- `crates/codeless-runtime/src/rpc/assistant_planner.rs` — planner
  prompt builder consumes the preset list; snapshot test covers all
  seven variants.
- `ui/codeless-ui/src/lib/policy/presets.{ts,test.ts}` — preset
  table lifted out of the composer module so the planner card and
  the composer share one source.
- `ui/codeless-ui/src/modules/jobs/composer/{JobComposer.tsx,index.ts}`
  — re-import from the new shared module.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx`
  — `draft_job` card embeds the policy picker; `update` card
  dispatches `set_job_policy` with the paused-job rule.
- `ui/codeless-ui/src/modules/assistant/AssistantThreadView.setPolicy.test.tsx`
  (new) — round-trip coverage for the picker on draft + update cards.

Layer-1 invariants:

- R1 (crate dependency direction): the two new Rust files live in
  `codeless-runtime` (host-only). Grep for `std::process` /
  `tokio::process` / `process::Command` in `auto_bypass_presets.rs`,
  `auto_bypass_failure_card.rs`, and the planner diff returns zero
  matches. No mobile-safe crate (`-types`, `-rpc`, `-client`) gained
  a host-only dependency. Holds.
- R2 (single transport): grep of `@tauri-apps` under
  `ui/codeless-ui/src/modules/assistant` returns zero. New imports
  route through `@/lib/rpc`, `@/lib/policy/presets`, or pure UI
  primitives. Every server call still goes through `rpc.call(...)`.
  Holds.
- R3 (one UI framework): no `*.web.tsx` / `*.mobile.tsx` /
  `*.ios.tsx` files added; the new `lib/policy/presets.ts` is one
  concept per file. Holds.
- R4 (SQLite source of truth): the failure-time card writes through
  `store.insert_assistant_message` + `store.touch_assistant_thread`
  before any bus publish; emit failures downgrade to
  `tracing::warn` and never poison the terminal-state path. The
  picker on the `update` card calls `set_job_policy` (existing RPC),
  not a client-side state mutation. Holds.
- R5 (single trust boundary): bearer token unchanged; the planner
  cards and the failure-time card use the same RPC surface as the
  rest of the assistant. No per-job / per-thread auth scopes
  introduced. Holds.
- Wire formats: `git diff 9d53514..13866ec -- crates/codeless-types
  crates/codeless-rpc 'crates/**/migrations/**' 'crates/**/*.sql'`
  returns empty. The W3 stages reuse the existing
  `AssistantAction::SetPolicy` and `set_job_policy` shapes. Holds.

Acceptance list (SCOPE-ASSISTANT-PARITY.md):

- Streamdown + token streaming on the assistant thread — landed in
  W1 and asserted by `CommonChat.parity.test.tsx`.
- `SubmitJobDialog` and `draft_job` render the same composer — W2
  embeds `JobComposer` in the editable branch; W3c adds the policy
  picker so the embedded form covers the seventh field too.
- Planner can recommend / change `AutoBypassPolicy` through the
  card surface — W3a/b/c thread the preset list through the
  planner prompt and the card UI; W3d covers the failure-time
  recommendation path.
- `CommonChat` is a component, not a switch — W1d's DOM-parity
  test pins the shared message-list renderer for both thread
  kinds.

Verdict: PASS.

PASS: W3 policy-card surface and failure-time recommender land
within R1/R2/R3/R4/R5 with no wire-format or schema changes, and
the four SCOPE-ASSISTANT-PARITY.md acceptance bullets are all
covered by the W1+W2+W3 commit series.
