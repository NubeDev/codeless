import type { EventEnvelope } from "@/lib/rpc";

// Wire-shape for one persisted chat row. Both `JobChat` (loading
// `CHAT.md`) and `AssistantThreadView` (loading `list_assistant_messages`)
// project into this shape before handing the renderer the history.
// Loader-agnostic by design — `SCOPE-ASSISTANT-PARITY.md` §W1 calls
// out the history loader as a wrapper concern, not the renderer's.
export type ChatMessageRole = "user" | "assistant";

export interface ChatMessage {
  role: ChatMessageRole;
  text: string;
  ts: string;
}

// Additional rows the chat feed renders alongside user/assistant
// bubbles. Sourced from the event stream so the user sees what the
// agent is *doing* (`Read foo.rs`, `Edit bar.rs`) and the inflection
// points (stage started, verify passed, job stopped, resumed) without
// having to leave the chat for the right-pane Timeline tab.
//
// Each item carries its event cursor for dedupe (events can replay on
// SSE reconnect) and the source `created_at` so the chronological
// merge with `ChatMessage.ts` stays correct.
export type LiveFeedItem =
  | {
      kind: "tool_call";
      cursor: number;
      created_at: number;
      tool: string;
      args_json: string;
    }
  | {
      kind: "lifecycle";
      cursor: number;
      created_at: number;
      label: string;
      tone: "neutral" | "good" | "bad" | "warn";
    };

export type ChatFeedRow =
  | { kind: "message"; message: ChatMessage; ts: number }
  | (LiveFeedItem & { ts: number });

// Translate one event envelope into a feed item (or `null` if the
// event has no chat-feed representation). The streaming-token and
// task-state events are handled separately by the chat surface; only
// signal that's interesting to the *user* belongs here.
export function liveItemFromEvent(env: EventEnvelope): LiveFeedItem | null {
  const e = env.event;
  const created_at = env.created_at;
  const cursor = env.cursor;
  switch (e.type) {
    case "tool-call":
    case "tool-approval-requested":
      return {
        kind: "tool_call",
        cursor,
        created_at,
        tool: e.tool,
        args_json: e.args_json,
      };
    case "stage-started":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label:
          typeof e.ordinal === "number"
            ? `stage ${e.ordinal + 1} started: ${e.name}`
            : `stage started: ${e.name}`,
        tone: "neutral",
      };
    case "stage-completed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: `stage ${e.status}`,
        tone: e.status === "passed" ? "good" : "bad",
      };
    case "verify-started":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "verify started",
        tone: "neutral",
      };
    case "verify-passed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "verify passed",
        tone: "good",
      };
    case "verify-failed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: `verify failed (exit ${e.exit_code})`,
        tone: "bad",
      };
    case "job-started":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "job started",
        tone: "good",
      };
    case "job-completed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "job completed",
        tone: "good",
      };
    case "job-stopped":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: `stopped: ${e.reason}`,
        tone: e.reason === "user" ? "neutral" : "warn",
      };
    case "job-paused":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: e.reason === "user" ? "paused" : `paused: ${e.reason}`,
        tone: "warn",
      };
    case "job-failed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "job failed",
        tone: "bad",
      };
    case "job-resumed":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: e.previous_reason
          ? `resumed (was ${e.previous_reason})`
          : "resumed",
        tone: "good",
      };
    case "review-requested":
      return {
        kind: "lifecycle",
        cursor,
        created_at,
        label: "review requested",
        tone: "warn",
      };
    default:
      return null;
  }
}

// Merge chat history (user/assistant bubbles persisted by whichever
// loader the wrapper supplied) with live feed items (tool calls,
// lifecycle dividers from the event stream) into one chronologically
// ordered list. The chat surface renders this directly; the in-flight
// streaming bubble is appended separately at the tail.
//
// Sort is stable on equal timestamps: history first, then live items.
// This keeps a conversation that opens with a user message from
// accidentally rendering a `job-started` divider above it.
export function mergeChatFeed(
  history: ChatMessage[],
  live: LiveFeedItem[],
): ChatFeedRow[] {
  const rows: ChatFeedRow[] = [];
  for (const m of history) {
    rows.push({ kind: "message", message: m, ts: chatTsToMs(m.ts) });
  }
  for (const item of live) {
    rows.push({ ...item, ts: item.created_at });
  }
  rows.sort((a, b) => a.ts - b.ts);
  return rows;
}

// `ChatMessage.ts` is an ISO string (or empty for the in-flight
// streaming bubble). Translate to ms-since-epoch for the merge sort;
// an unparseable / empty value sorts as 0 which puts it at the top —
// the right place for the very first user message of a freshly-opened
// conversation.
export function chatTsToMs(iso: string): number {
  if (!iso) return 0;
  const n = Date.parse(iso);
  return Number.isFinite(n) ? n : 0;
}
