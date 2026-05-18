## Done

- Extracted shared chat chrome from `JobChat` (in `ui/codeless-ui/src/modules/jobs/RunPane.tsx`) into the `modules/chat` package: new files `ChatBubble.tsx`, `ToolCallCard.tsx`, `LifecycleDivider.tsx`, `PulseDot.tsx`, `feed.ts` (types + `liveItemFromEvent`/`mergeChatFeed`/`chatTsToMs`), `format.ts` (`wallClockTime`/`prettyJson`).
- Added the new `ChatMessageList` renderer (`modules/chat/ChatMessageList.tsx`) which owns `useEventStream` against a caller-supplied `EventFilter`, the `activeTaskId`-gated streaming bubble (Streamdown via `ChatBubble`), live-feed merge, scroll anchoring, and a `since` cursor knob; loader-agnostic per SCOPE §W1.
- Updated `modules/chat/index.ts` to export the new surfaces; `RunPane.tsx` now imports `ChatBubble`/`ToolCallCard`/`LifecycleDivider`/`PulseDot`/`mergeChatFeed`/`liveItemFromEvent`/`ChatMessage`/`LiveFeedItem` from `../chat` and the inline duplicates are deleted (RunPane shrinks ~512 lines).
- Added `modules/chat/ChatMessageList.test.tsx` covering: (a) `ai-token` deltas for the active task accumulate into the in-flight bubble, (b) deltas tagged with a different `task_id` are ignored, (c) the bubble is hidden when `activeTaskId == null`.
- Committed as `c0691cf` on `feat/assistant-parity` with message `W1a lift streaming and Streamdown and tool-card chrome out of JobChat into CommonChat`. Workspace constraint greps clean: no new `@tauri-apps/*` imports under `modules/{chat,assistant,jobs}`, no `.web.tsx`/`.desktop.tsx`/`.mobile.tsx` files anywhere.
- `pnpm typecheck` clean. `pnpm test -- --run` clean: 63/63 tests pass (was 60; +3 new streaming cases).

## Next

- Stage W1b — shrink `JobChat` and `AssistantThreadView` to wrappers that mount `<ChatMessageList />` with a history-loader prop (CHAT.md vs `list_assistant_messages`). JobChat keeps composer/send/attach; AssistantThreadView keeps title bar + composer + action-card path.

## What you need to know

- `ChatMessageList` is intentionally not yet consumed by `JobChat`/`AssistantThreadView` — W1a only lifts; W1b switches the wrappers. `WORKFLOW.md` explicitly says do not batch W1a + W1b.
- `PulseDot` was relocated to `modules/chat/PulseDot.tsx` because both `ChatBubble` (in the chat module now) and several existing call sites in `RunPane.tsx` (lines around 729/3324/3375/3477 in pre-extract numbering, now slightly lower) need it. RunPane imports it from `../chat`.
- `MarkdownBubble.tsx` (used by `AssistantThreadView` for plain prose) is unchanged; `ChatBubble.tsx` is the richer Streamdown bubble with role header + timestamp + streaming pulse that `ChatMessageList` mounts. W1b can decide whether to unify them; for now both exist side-by-side.
- The pre-existing uncommitted noise in the worktree (`Cargo.toml`, `crates/codeless-adapters-host/src/lib.rs`, `src/lib/rpc/methods.ts`, `vite.config.ts`, untracked `runs/`, `demos/`, `index.desktop.html`, etc.) was already present at session start and is unrelated to assistant-parity; only my stage W1a files were staged + committed.
- No session doc was created — prior W2a/W2b/REVIEW ticks did not create one either; the durable record for W1 reviews continues in `.codeless/jobs/assistant-parity/REVIEWS.md` (next REVIEW gate is after W1d).

## Open questions

- The scope text says "`CommonChat` becomes the real renderer" but in W1a the existing `CommonChat.tsx` discriminated-union switch is untouched (still routes to `JobChat`/`AiChatView`/`AssistantThreadView`). The `kind`-discriminated facade is retired only when both wrappers shrink (W1b) and the `kind` slot becomes unnecessary. If W1b's author prefers to fold `CommonChat.tsx` itself into `ChatMessageList` rather than have wrappers mount it, the existing `CommonChat.test.tsx` will need updating — flag at W1b kickoff.
