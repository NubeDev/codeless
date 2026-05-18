## Done

- Created `ui/codeless-ui/src/modules/jobs/ChatTab.tsx`: rewritten over the new job-chat RPCs with a module-scoped SWR-style cache keyed on `job_id`, rehydrating from `list_job_messages` on mount and appending from `chat-message-appended` on the existing `useEventStream` subscription pool. `MessageId` is the dedup gate.
- Created `ui/codeless-ui/src/modules/jobs/ChatTab.test.tsx` with a fixture `RpcClient`. The `chat_tab_round_trips_a_post` case asserts input → `post_job_message` → `ChatMessageAppended` → render without touching the network; three additional cases cover rehydration, non-origin Telegram fan-out, and MessageId dedup.
- Updated `crates/codeless-rpc/examples/wire_ts.rs` to register `ChatMessage`/`ChatBinding`/`ChatTransport`/`ChatRole`/`MessageId`/`PostJobMessageArgs`/`ListJobMessagesArgs`/`ListJobMessagesResult`/`BindChatThreadArgs`, and regenerated `ui/codeless-ui/src/lib/rpc/generated/wire.ts` so the UI picks up those types (the per-crate snapshot tests already pinned them, but the combined exporter the UI consumes had not).
- Added `post_job_message` / `list_job_messages` / `bind_chat_thread` entries to `RpcMethodMap` in `lib/rpc/methods.ts`.
- `pnpm vitest run` green (24 files / 122 tests). `pnpm exec tsc --noEmit` green. `cargo check --workspace` green. Both specta snapshot tests green.
- Committed as `168895d` on `codeless/job-chat`.

## Next

- (none) — stage 7 ("Telegram adapter — transport-side: poll updates, decode `/codeless bind`, forward inbound to `post_job_message`…") is for a fresh session.

## What you need to know

- The legacy `JobChat` in `RunPane.tsx` (which parses `CHAT.md` out of the worktree and keeps an optimistic local list) is still wired through `CommonChat` → `JobChatPage`. `ChatTab.tsx` is a fresh component and is not yet plugged into the page; integrating/removing the legacy chat is left for a later stage of the JOB-CHAT phase. The stage prompt only asked for the rewrite of `ChatTab.tsx`, the RPC plumbing, and the round-trip vitest.
- The cache exposes `__resetChatCacheForTests()` so test runs do not cross-pollinate module-scoped state across mounts.
- The vitest fixture client implements `subscribeWithState` (preferred by `joinSubscription`); the iterable `subscribe` is a no-op stub.
- Cargo invocations from this worktree require `../../ai-runner/Cargo.toml`'s `workspace = "..."` to point at this worktree. I flipped it to run `cargo run -p codeless-rpc --example wire_ts`, the snapshot tests, and `cargo check --workspace`, then restored it to its prior `job-01KRX4ZPF10J3QZ35R5GK8336X` value before committing. If the next session needs to invoke cargo on this worktree, it will need to do the same dance (the ai-runner Cargo.toml is not in this repo — it sits in the `ai-runner` worktree).

## Open questions

- (none)
