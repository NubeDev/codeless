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
