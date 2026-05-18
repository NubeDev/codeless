## Done

- W1b landed on feat/assistant-parity as 0ee0305 (pushed).
- ChatMessageList grew two opt-ins: `renderMessage(message, key)` to override row rendering for surfaces with richer row types, and a `"*"` activeTaskId sentinel for surfaces whose RPC doesn't expose the planner's task id. ChatMessage gained optional `key` + `meta` for wrappers to carry their native row type through the projection.
- JobChat shrank: dropped its inline `<ul>`/mergeChatFeed/streaming-bubble, replaced local streaming state with a `activeTaskId: string | null`, mounted `<ChatMessageList filter={…} history={history} activeTaskId={activeTaskId} autoScroll={autoScroll} />`. Kept its own `useEventStreamWithState` (single EventSource shared with ChatMessageList via the hook pool) for the AgentActivityIndicator's sseStatus + lastEventAt.
- AssistantThreadView shrank: dropped its `useEventStream`, `streamingText`/`streamingActive`, `scrollAnchorRef`, and the ScrollArea-based inline `messages.map`. It now projects `AssistantMessage[]` → `ChatMessage[]` (keyed by `m.id`, original attached via `meta`) and mounts `<ChatMessageList filter={…} activeTaskId={sending ? "*" : null} renderMessage={…}>` whose `renderMessage` callback dispatches to the existing MessageBubble (action / attachment / tool dispatch unchanged). `useMemo` for the projection; `MarkdownBubble` import retained for prose dispatch inside MessageBubble.
- Note: a separate commit landed during the session ("completed more stages", 972b34c) ahead of my W1b commit; it already absorbed the RunPane.tsx / ChatMessageList / feed.ts edits I made earlier in the stage. My final mani commit (0ee0305) carried the AssistantThreadView shrink plus the `<li>` wrapper safety fix in ChatMessageList.

## Next

- W1c retires `focusStore.refreshTick` once the rail subscribes to the planner thread-touched envelope. `AssistantThreadView` still consumes `refreshTick` via `useAssistantFocus((s) => s.refreshTick)` (line ~57) to force a re-list — that subscription is the W1c target, not anything in this stage.
- W1d adds the parity test asserting identical message-list DOM for job vs assistant threads, then the REVIEW gate before W3.

## What you need to know

- `ChatMessageList` renders a `<ul>`. When `renderMessage` is supplied, the renderer wraps the caller's output in `<li>` so card markup (`<div>`-based MessageBubble) stays valid HTML. ChatBubble self-wraps in `<li>` and bypasses this path.
- The `"*"` sentinel on `activeTaskId` accepts every event regardless of `env.task_id`. It is safe because the underlying RPC (`append_assistant_message`) blocks per-thread, so only one in-flight turn can be streaming at a time. JobChat does not use the sentinel — it gets the real task id from `agent_chat`'s return.
- The assistant view lost its radix `<ScrollArea>` wrapper; ChatMessageList's `<ul class="overflow-y-auto …">` handles scrolling. If the visual diff matters for the REVIEW gate, the wrapper passes a `className` slot — caller can re-skin without forking the renderer.
- `useEventStream` pools connections by filter so JobChat's status subscription + ChatMessageList's accumulator subscription share one EventSource per job.
- `tsc --noEmit` clean; `pnpm test -- --run` → 13 files / 63 tests passed (4.33s); `pnpm lint` is a no-op script. The W2b round-trip test (`AssistantThreadView.draftJob.test.tsx`) still passes against the rewritten wrapper, confirming the card dispatch path survived the shrink.

## Open questions

- (none)
