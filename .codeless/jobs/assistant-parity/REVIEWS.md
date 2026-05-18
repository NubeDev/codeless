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
