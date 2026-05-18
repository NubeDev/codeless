## Done

- Examined the diff cc76d14..df0faa9 (W2a `c7af399`, W2b `df0faa9`); five files changed, all UI / job-doc only.
- Verified Layer-1 invariants against the diff:
- R1: no Rust files touched, no `std::process` / `tokio::process` introduced.
- R2: `AssistantThreadView.tsx` adds imports only from `react`, `@/lib/rpc`, shadcn primitives, `@/lib/utils`, `@/lib/route`, `../chat`, `../jobs/composer`; no `@tauri-apps/*`; every server call routes through `useRpc()` (`rpc.call(...)`, `rpc.serverInfo()`).
- R4: optimistic "confirmed" flip + synthetic tool row are presentation-only and the source already comments that the persisted server card stays `pending` pending a W3 endpoint.
- R5: no per-thread or per-job auth scopes added.
- Wire formats untouched: no Rust types, RPC methods, or migrations changed; composer mapping reuses existing `SubmitJobArgs` and `AssistantAction { tool: "draft_job" }` shapes.
- Recorded verdict in `.codeless/jobs/assistant-parity/REVIEWS.md` and committed (`68679c4`) with the stage title.

## Next

- Stage W1a: lift streaming, Streamdown, and tool-card chrome out of `JobChat` into `CommonChat` (per `.codeless/jobs/assistant-parity/SCOPE.md` §W1 and parity doc §W1 touch points).

## What you need to know

- PASS: W2a/W2b leave R1/R2/R4/R5 intact and introduce no new wire shapes; the composer round-trip is wired entirely through `RpcClient`.
- The worktree carries unstaged pre-existing edits (Cargo.toml, runtime adapters, `index.desktop.html`, etc.) that are NOT from W2a/W2b and were intentionally left untouched — review scope was the commits, not the dirty sandbox state. Note `ui/codeless-ui/index.desktop.html` is an untracked per-shell file; if it lands in a future commit it would trip R3.
- `DOCS/sessions/2026-05-XX-assistant-parity.md` was not created at W2a despite WORKFLOW.md prescribing it; out of scope for this REVIEW but worth flagging for the next WORK stage to create.

## Open questions

- (none)
