// The single web entry point onto the JOB-CHAT.md substrate. Every
// row this component renders lives in `chat_messages` on the server;
// every row this component posts goes through `post_job_message`.
// There is no transport-local message store any more — the legacy
// `JobChat` in `RunPane.tsx` parsed `CHAT.md` out of the worktree and
// kept its own optimistic list, but that path is incompatible with
// the multi-transport invariant (a Telegram inbound has to land in
// the same thread as the web composer), so it does not survive here.
//
// SWR-style cache. `chatCache` is module-scoped, keyed on `job_id`,
// rehydrated from `list_job_messages` on the first mount that names a
// given job, and reused on subsequent mounts so re-opening the tab
// does not blank the conversation while the network round-trips. New
// rows arrive exclusively via the `chat-message-appended` event from
// the SSE stream; the local post path does not optimistically append
// because the server is the single source of truth for ordering and
// the round-trip is short. The asymmetric echo-suppression rule
// (JOB-CHAT.md "Transport adapters") lives in `codeless-bot-core`,
// not here — the web client renders every append regardless of
// transport so a Telegram-originated message is visible in the
// browser tab without ceremony.
//
// `MessageId` is a ULID and the de-dup gate; the cache builds a
// Set<MessageId> on each merge so a redelivered event (server
// restart, multi-tab race, SSE replay) never double-renders.

import { useCallback, useEffect, useRef, useState } from "react";

import { useEventStream, useRpc } from "@/lib/rpc";
import type {
  ChatMessage,
  EventEnvelope,
  JobId,
  MessageId,
  RpcClient,
} from "@/lib/rpc";

const PAGE_LIMIT = 200;

interface CacheEntry {
  messages: ChatMessage[];
  ids: Set<MessageId>;
  hydrated: boolean;
  inflight: Promise<void> | null;
}

const chatCache = new Map<JobId, CacheEntry>();
const listeners = new Map<JobId, Set<() => void>>();

function getEntry(jobId: JobId): CacheEntry {
  let entry = chatCache.get(jobId);
  if (!entry) {
    entry = { messages: [], ids: new Set(), hydrated: false, inflight: null };
    chatCache.set(jobId, entry);
  }
  return entry;
}

function notify(jobId: JobId): void {
  const set = listeners.get(jobId);
  if (!set) return;
  for (const fn of set) fn();
}

function appendIfMissing(entry: CacheEntry, msg: ChatMessage): boolean {
  if (entry.ids.has(msg.id)) return false;
  entry.ids.add(msg.id);
  // Server returns oldest-first within a page (per ListJobMessagesResult);
  // events arrive in insertion order; either way the new row belongs at
  // the tail. A late-arriving older message is possible on a back-paged
  // fetch — we splice by created_at then by id as the tiebreaker so the
  // rendered list stays monotonic.
  const last = entry.messages[entry.messages.length - 1];
  if (
    !last ||
    msg.created_at > last.created_at ||
    (msg.created_at === last.created_at && msg.id > last.id)
  ) {
    entry.messages.push(msg);
  } else {
    const idx = entry.messages.findIndex(
      (m) =>
        m.created_at > msg.created_at ||
        (m.created_at === msg.created_at && m.id > msg.id),
    );
    entry.messages.splice(idx, 0, msg);
  }
  return true;
}

function rehydrate(rpc: RpcClient, jobId: JobId): Promise<void> {
  const entry = getEntry(jobId);
  if (entry.hydrated) return Promise.resolve();
  if (entry.inflight) return entry.inflight;
  entry.inflight = rpc
    .call("list_job_messages", { job_id: jobId, limit: PAGE_LIMIT })
    .then((res) => {
      for (const msg of res.messages) appendIfMissing(entry, msg);
      entry.hydrated = true;
      entry.inflight = null;
      notify(jobId);
    })
    .catch((err) => {
      entry.inflight = null;
      throw err;
    });
  return entry.inflight;
}

function ingestEvent(jobId: JobId, env: EventEnvelope): void {
  const ev = env.event;
  if (ev.type !== "chat-message-appended") return;
  if (ev.job_id !== jobId) return;
  const entry = getEntry(jobId);
  if (appendIfMissing(entry, ev.message)) notify(jobId);
}

// Test-only reset. Vitest mounts each test in a shared module graph; a
// leaking entry from a previous test would cross-pollinate the cache
// and mask a real regression. The reset is not exposed from the
// public `modules/jobs` barrel — it lives on the file so the test can
// import it by relative path and nothing else.
export function __resetChatCacheForTests(): void {
  chatCache.clear();
  listeners.clear();
}

interface UseJobMessages {
  messages: ChatMessage[];
  hydrated: boolean;
  error: Error | null;
}

function useJobMessages(jobId: JobId): UseJobMessages {
  const rpc = useRpc();
  const [, setRev] = useState(0);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    let set = listeners.get(jobId);
    if (!set) {
      set = new Set();
      listeners.set(jobId, set);
    }
    const fn = () => setRev((n) => n + 1);
    set.add(fn);
    rehydrate(rpc, jobId).catch((e: unknown) => {
      setError(e instanceof Error ? e : new Error(String(e)));
    });
    return () => {
      set!.delete(fn);
      if (set!.size === 0) listeners.delete(jobId);
    };
  }, [rpc, jobId]);

  const onEvent = useCallback(
    (env: EventEnvelope) => ingestEvent(jobId, env),
    [jobId],
  );
  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  const entry = getEntry(jobId);
  return { messages: entry.messages, hydrated: entry.hydrated, error };
}

export interface ChatTabProps {
  jobId: JobId;
  // Display name attached to outbound `post_job_message` calls. The
  // server requires `author` non-empty; the page header already knows
  // the operator handle so it threads it down rather than re-fetching
  // here.
  author?: string;
}

export function ChatTab({ jobId, author = "web" }: ChatTabProps) {
  const rpc = useRpc();
  const { messages, hydrated, error } = useJobMessages(jobId);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Autoscroll to the tail on new rows. A user scrolled-up should
  // not get yanked back; the simple test here is whether the user
  // was within one viewport of the tail before the append.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom =
      el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight;
    if (nearBottom) el.scrollTop = el.scrollHeight;
  }, [messages.length]);

  const send = async () => {
    const body = input.trim();
    if (!body || sending) return;
    setSending(true);
    setSendError(null);
    try {
      await rpc.call("post_job_message", {
        job_id: jobId,
        transport: "web",
        author,
        role: "user",
        body,
      });
      setInput("");
    } catch (e) {
      setSendError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        ref={scrollRef}
        data-testid="chat-tab-messages"
        className="flex-1 min-h-0 overflow-y-auto px-4 py-3"
      >
        {!hydrated && messages.length === 0 && (
          <div className="text-muted-foreground text-sm">Loading…</div>
        )}
        {error && (
          <div className="text-destructive text-sm">
            failed to load chat: {error.message}
          </div>
        )}
        <ul className="space-y-2">
          {messages.map((m) => (
            <li
              key={m.id}
              data-testid="chat-tab-message"
              data-message-id={m.id}
              data-role={m.role}
              data-transport={m.transport}
              className="rounded border px-3 py-2 text-sm"
            >
              <div className="text-muted-foreground mb-1 text-[11px]">
                <span className="font-mono">{m.author}</span>
                <span className="mx-1">·</span>
                <span>{m.transport}</span>
                <span className="mx-1">·</span>
                <span>{m.role}</span>
              </div>
              <div className="whitespace-pre-wrap">{m.body}</div>
            </li>
          ))}
        </ul>
      </div>
      <form
        className="border-t p-3"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <div className="flex gap-2">
          <textarea
            data-testid="chat-tab-input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Message this job…"
            disabled={sending}
            rows={2}
            className="bg-background flex-1 resize-none rounded border px-3 py-2 text-sm"
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
          />
          <button
            type="submit"
            data-testid="chat-tab-send"
            disabled={sending || input.trim().length === 0}
            className="bg-primary text-primary-foreground rounded px-3 py-2 text-sm disabled:opacity-50"
          >
            {sending ? "sending…" : "send"}
          </button>
        </div>
        {sendError && (
          <p className="text-destructive mt-2 text-xs">{sendError}</p>
        )}
      </form>
    </div>
  );
}
