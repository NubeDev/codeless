import {
  Fragment,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useEventStream, type EventEnvelope, type EventFilter } from "@/lib/rpc";

import { ChatBubble } from "./ChatBubble";
import { LifecycleDivider } from "./LifecycleDivider";
import { ToolCallCard } from "./ToolCallCard";
import {
  liveItemFromEvent,
  mergeChatFeed,
  type ChatMessage,
  type LiveFeedItem,
} from "./feed";

// Shared message-list renderer for every chat surface in the UI. The
// load-bearing extraction `SCOPE-ASSISTANT-PARITY.md` §W1 demands:
// streaming-token accumulator, Streamdown bubbles, and tool-card
// chrome all live here so a markdown-quirk fix in `ChatBubble` lands
// in the job chat and the assistant chat in the same commit.
//
// History is a prop: the wrapper supplies `parseChatMarkdown` against
// `CHAT.md` for jobs, `list_assistant_messages` for the assistant, an
// AI-SDK store snapshot for `AiChatView`. The renderer is loader-
// agnostic.
//
// Subscription is owned here. The `filter` prop lets the caller pin
// the channel: job chats pass `{ scope: "job", job_id }`; the
// assistant passes `{ scope: "job", job_id: thread_id }` because the
// planner publishes on the thread's id reused as a synthetic
// `JobId`. `since` defaults to `0` (replay the full thread on
// subscribe) — callers that need live-only state pass a non-zero
// cursor.
//
// `activeTaskId` gates the in-flight streaming bubble. When the
// caller's send-handler has just kicked off an `agent_chat` /
// `append_assistant_message`, it sets the task id; the renderer
// accumulates matching `ai-token` deltas into a bubble at the tail
// of the list and clears the bubble on the next `ai-message-complete`
// for that task (or when the parent unsets `activeTaskId` after the
// awaited RPC returns the persisted message). A null task id keeps
// the bubble hidden — replayed events from earlier turns don't
// surface as phantom in-flight prose.
//
// The sentinel `"*"` accepts every event regardless of `task_id` and
// is used by surfaces whose underlying RPC doesn't expose the task
// id the planner publishes on (e.g. `append_assistant_message`, which
// blocks until the turn completes and never round-trips the task id
// to the client). It is safe because the same RPC contract pins one
// in-flight turn per thread on the server.
export type ChatMessageListProps = {
  filter: EventFilter;
  history: ChatMessage[];
  activeTaskId: string | null;
  /**
   * Override the default `<ChatBubble />` row renderer. The assistant
   * thread mounts this to dispatch on `AssistantMessage` shape — action
   * cards, attachment cards, tool-result rows. The wrapper attaches
   * the original row via `ChatMessage.meta` and reads it back here.
   * Receives the projected message plus the stable key the renderer
   * would otherwise have generated, so the wrapper can pass it through
   * unchanged when it returns a custom element.
   */
  renderMessage?: (message: ChatMessage, key: string) => ReactNode;
  /**
   * Initial subscription cursor. Defaults to `0` so a freshly opened
   * surface replays the full thread (matching what `JobTimeline` and
   * the existing job chat do). Long-lived surfaces that already have
   * a captured cursor pass it here to skip the replay.
   */
  since?: number;
  /**
   * Pin the feed to the bottom as it grows. Defaults to true — every
   * existing chat surface defaults to autoscroll; callers that want a
   * scrollback experience flip this off.
   */
  autoScroll?: boolean;
  /**
   * Optional empty-state slot rendered when history is empty and no
   * streaming bubble / live items have arrived yet. `null` (default)
   * renders nothing.
   */
  emptyState?: ReactNode;
  /**
   * Notified each time the renderer accepts an envelope from its
   * subscription. The job chat reads this to drive the agent-activity
   * pill; surfaces without that pill omit it.
   */
  onEventReceived?: (env: EventEnvelope) => void;
  /**
   * Optional extra className applied to the scrolling `<ul>`. Lets
   * the wrapper colour the container without forking the renderer.
   */
  className?: string;
};

export function ChatMessageList({
  filter,
  history,
  activeTaskId,
  renderMessage,
  since = 0,
  autoScroll = true,
  emptyState,
  onEventReceived,
  className,
}: ChatMessageListProps) {
  const [liveItems, setLiveItems] = useState<LiveFeedItem[]>([]);
  // Streaming buffer for the in-flight assistant turn. Distinguished
  // by `taskId` so a stray event for an earlier turn cannot pollute
  // the current bubble. Cleared either when the activeTaskId flips
  // away (parent awaited the RPC) or when an `ai-message-complete`
  // arrives — whichever lands first.
  const [streamingText, setStreamingText] = useState("");
  const [streamingActive, setStreamingActive] = useState(false);
  const scrollRef = useRef<HTMLUListElement | null>(null);

  // Resetting when the filter target changes prevents tokens from a
  // prior thread bleeding into the new transcript. The filter is
  // serialised so a job-id swap rebuilds the buffer; identity churn
  // on the object itself does not.
  const filterKey = JSON.stringify(filter);
  useEffect(() => {
    setLiveItems([]);
    setStreamingText("");
    setStreamingActive(false);
  }, [filterKey]);

  // Clear the in-flight bubble whenever the parent retracts the
  // active task id. That signals the awaited RPC returned and the
  // persisted message is already in `history`; leaving the buffer
  // behind would double-render the same prose.
  useEffect(() => {
    if (activeTaskId == null) {
      setStreamingText("");
      setStreamingActive(false);
    }
  }, [activeTaskId]);

  const onEvent = useCallback(
    (env: EventEnvelope) => {
      onEventReceived?.(env);
      const e = env.event;
      const taskMatches =
        activeTaskId != null
        && (activeTaskId === "*" || env.task_id === activeTaskId);
      if (taskMatches) {
        if (e.type === "ai-token") {
          setStreamingText((prev) => prev + e.delta);
          setStreamingActive(true);
        } else if (e.type === "ai-message-complete") {
          // Completion handshake — the awaited RPC result will arrive
          // imminently with the persisted final message; freezing the
          // pulse here avoids a flicker between the last token and
          // the bubble being replaced by the persisted row.
          setStreamingActive(false);
        }
      }
      const item = liveItemFromEvent(env);
      if (item) {
        setLiveItems((prev) =>
          prev.some((p) => p.cursor === item.cursor) ? prev : [...prev, item],
        );
      }
    },
    [activeTaskId, onEventReceived],
  );

  useEventStream(filter, onEvent, since);

  useEffect(() => {
    if (!autoScroll) return;
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [autoScroll, history, liveItems, streamingText]);

  const rows = mergeChatFeed(history, liveItems);
  const isEmpty =
    rows.length === 0 && streamingText.length === 0 && activeTaskId == null;

  return (
    <ul
      ref={scrollRef}
      className={
        className ?? "min-h-0 flex-1 space-y-2 overflow-y-auto pr-1"
      }
    >
      {isEmpty && emptyState}
      {rows.map((row, i) => {
        if (row.kind === "message") {
          const key = row.message.key ?? `m-${i}`;
          if (renderMessage) {
            return (
              <Fragment key={key}>
                {renderMessage(row.message, key)}
              </Fragment>
            );
          }
          return <ChatBubble key={key} message={row.message} />;
        }
        if (row.kind === "tool_call") {
          return (
            <ToolCallCard
              key={`t-${row.cursor}`}
              tool={row.tool}
              argsJson={row.args_json}
              ts={row.created_at}
            />
          );
        }
        return (
          <LifecycleDivider
            key={`l-${row.cursor}`}
            label={row.label}
            tone={row.tone}
            ts={row.created_at}
          />
        );
      })}
      {activeTaskId != null && (
        <ChatBubble
          message={{
            role: "assistant",
            text: streamingText || "…",
            ts: "",
          }}
          streaming={streamingActive}
        />
      )}
    </ul>
  );
}
