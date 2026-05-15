import { useCallback, useEffect, useRef, useState } from "react";
import {
  useRpc,
  type AssistantAction,
  type AssistantActionCard,
  type AssistantActionStatus,
  type AssistantMessage,
  type AssistantThread,
} from "@/lib/rpc";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

// Stage-6 assistant view. Renders the persisted transcript for one
// thread plus a composer that appends a user turn and the no-op
// server-side responder. The renderer is deliberately minimal: full
// markdown / tool-call / attachment rendering reuses the JobChat
// machinery in later stages once the assistant grows the matching
// server-side capabilities. Keeping this surface small now means the
// rewire to share chrome with JobChat does not have to undo a richer
// renderer first.
export type AssistantThreadViewProps = {
  thread: AssistantThread;
  /**
   * Fired after a successful `append_assistant_message` so the parent
   * rail can refresh `updated_at` ordering. Optional — the view still
   * works without it; the rail just won't re-sort until the next
   * refresh.
   */
  onThreadTouched?: () => void;
};

export function AssistantThreadView({
  thread,
  onThreadTouched,
}: AssistantThreadViewProps) {
  const rpc = useRpc();
  const [messages, setMessages] = useState<AssistantMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const scrollAnchorRef = useRef<HTMLDivElement | null>(null);

  // Reload when the parent rail swaps in a different thread.
  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setErr(null);
    void rpc
      .call("list_assistant_messages", { thread_id: thread.id })
      .then((res) => {
        if (cancelled) return;
        setMessages(res.messages);
        setLoaded(true);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setErr(e instanceof Error ? e.message : String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, thread.id]);

  // Scroll to the bottom on new messages — matches every other chat
  // surface and keeps the latest turn in view without the user having
  // to drag the scrollbar.
  useEffect(() => {
    scrollAnchorRef.current?.scrollIntoView({ block: "end" });
  }, [messages.length]);

  const onSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      const content = input.trim();
      if (!content || sending) return;
      setSending(true);
      setErr(null);
      try {
        const res = await rpc.call("append_assistant_message", {
          thread_id: thread.id,
          content,
        });
        setMessages((prev) => [
          ...prev,
          res.user_message,
          res.assistant_message,
        ]);
        setInput("");
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setSending(false);
      }
    },
    [rpc, thread.id, input, sending, onThreadTouched],
  );

  // Action-card resolution: confirm dispatches the proposed tool call
  // server-side and appends a `Tool`-role message with the structured
  // result; cancel only flips status. Both replace the card row in
  // place (same `id`, new `meta_json`) so React state stays consistent
  // with the rail without a full re-list.
  const onConfirmAction = useCallback(
    async (messageId: string) => {
      setErr(null);
      try {
        const res = await rpc.call("confirm_assistant_action", {
          thread_id: thread.id,
          message_id: messageId as AssistantMessage["id"],
        });
        setMessages((prev) => {
          const next = prev.map((m) =>
            m.id === res.card.id ? res.card : m,
          );
          next.push(res.tool_message);
          return next;
        });
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      }
    },
    [rpc, thread.id, onThreadTouched],
  );

  const onCancelAction = useCallback(
    async (messageId: string) => {
      setErr(null);
      try {
        const res = await rpc.call("cancel_assistant_action", {
          thread_id: thread.id,
          message_id: messageId as AssistantMessage["id"],
        });
        setMessages((prev) =>
          prev.map((m) => (m.id === res.card.id ? res.card : m)),
        );
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      }
    },
    [rpc, thread.id, onThreadTouched],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="border-b border-border/60 px-4 py-2">
        <h2 className="truncate text-sm font-medium">{thread.title}</h2>
        <p className="text-[11px] text-muted-foreground">
          {thread.id}
        </p>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-3 p-4">
          {!loaded ? (
            <div className="text-xs text-muted-foreground">Loading…</div>
          ) : messages.length === 0 ? (
            <div className="text-xs text-muted-foreground">
              No messages yet. Say hello to seed the thread.
            </div>
          ) : (
            messages.map((m) => (
              <MessageBubble
                key={m.id}
                message={m}
                onConfirmAction={onConfirmAction}
                onCancelAction={onCancelAction}
              />
            ))
          )}
          <div ref={scrollAnchorRef} />
        </div>
      </ScrollArea>

      {err && (
        <div className="border-t border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive">
          {err}
        </div>
      )}

      <form
        onSubmit={onSubmit}
        className="flex items-end gap-2 border-t border-border/60 bg-card p-3"
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends; Shift+Enter inserts a newline. Matches the
            // other chat composers in the app so muscle memory transfers.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void onSubmit();
            }
          }}
          placeholder="Message the assistant…"
          rows={2}
          disabled={sending}
          className="min-h-[44px] flex-1 resize-none rounded-md border border-border/60 bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <Button type="submit" size="sm" disabled={!input.trim() || sending}>
          {sending ? "Sending…" : "Send"}
        </Button>
      </form>
    </div>
  );
}

type MessageBubbleProps = {
  message: AssistantMessage;
  onConfirmAction: (messageId: string) => void;
  onCancelAction: (messageId: string) => void;
};

function MessageBubble({
  message,
  onConfirmAction,
  onCancelAction,
}: MessageBubbleProps) {
  // Action cards are stored as `Assistant`-role messages whose
  // `meta_json` decodes to an `AssistantActionCard`. The role
  // discriminator stays "assistant" (not a new role) so renderers
  // that don't know about cards still see them as a normal turn
  // with a human-readable summary in `content`.
  const card = parseActionCard(message.meta_json);
  if (message.role === "assistant" && card) {
    return (
      <ActionCardView
        message={message}
        card={card}
        onConfirm={() => onConfirmAction(message.id)}
        onCancel={() => onCancelAction(message.id)}
      />
    );
  }
  if (message.role === "tool") {
    return <ToolResultView message={message} />;
  }
  const isUser = message.role === "user";
  return (
    <div
      className={cn(
        "flex w-full",
        isUser ? "justify-end" : "justify-start",
      )}
    >
      <div
        className={cn(
          "max-w-[85%] whitespace-pre-wrap rounded-md px-3 py-2 text-sm",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-foreground",
        )}
      >
        {message.content}
      </div>
    </div>
  );
}

// `meta_json` is the wire-typed `string | null`. Cards are JSON
// documents with `kind == "action_card"`; everything else is some
// other future meta shape and falls back to plain rendering.
function parseActionCard(raw: string | null): AssistantActionCard | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<AssistantActionCard>;
    if (parsed && parsed.kind === "action_card" && parsed.action && parsed.status) {
      return parsed as AssistantActionCard;
    }
    return null;
  } catch {
    return null;
  }
}

const STATUS_LABEL: Record<AssistantActionStatus, string> = {
  pending: "Pending",
  confirmed: "Confirmed",
  cancelled: "Cancelled",
  failed: "Failed",
};

const STATUS_TONE: Record<AssistantActionStatus, string> = {
  pending: "border-yellow-500/60 bg-yellow-500/5",
  confirmed: "border-emerald-500/60 bg-emerald-500/5",
  cancelled: "border-muted-foreground/40 bg-muted/40",
  failed: "border-destructive/60 bg-destructive/10",
};

type ActionCardViewProps = {
  message: AssistantMessage;
  card: AssistantActionCard;
  onConfirm: () => void;
  onCancel: () => void;
};

// Confirmation-gated action card. The user-facing "confirm" button is
// only live while `status == "pending"`; once a card is resolved the
// buttons retire so a re-render of the transcript cannot fire the
// same RPC twice (the server enforces this too — the UI is just
// cooperating).
function ActionCardView({
  message,
  card,
  onConfirm,
  onCancel,
}: ActionCardViewProps) {
  const isPending = card.status === "pending";
  return (
    <div className="flex justify-start">
      <div
        className={cn(
          "flex w-full max-w-[85%] flex-col gap-2 rounded-md border px-3 py-2 text-sm",
          STATUS_TONE[card.status],
        )}
      >
        <div className="flex items-center justify-between gap-2">
          <span className="text-[11px] font-mono uppercase tracking-wide text-muted-foreground">
            {actionLabel(card.action)}
          </span>
          <span className="text-[11px] uppercase text-muted-foreground">
            {STATUS_LABEL[card.status]}
          </span>
        </div>
        <div className="whitespace-pre-wrap">{message.content}</div>
        {isPending && (
          <div className="mt-1 flex justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={onCancel}
              aria-label="Cancel action"
            >
              Cancel
            </Button>
            <Button
              size="sm"
              onClick={onConfirm}
              aria-label="Confirm action"
            >
              Confirm
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

// Short `tool:method_name` label for the card header so the user can
// see what they are about to run without parsing the human summary.
function actionLabel(action: AssistantAction): string {
  return `tool:${action.tool}`;
}

// `Tool`-role messages carry the structured result of a confirmed
// action. The structured payload sits on `meta_json`; the
// human-readable summary is `content`. We render the summary plus a
// foldable raw-JSON block so an action's outcome is inspectable
// without a separate developer surface.
function ToolResultView({ message }: { message: AssistantMessage }) {
  return (
    <div className="flex justify-start">
      <div className="flex w-full max-w-[85%] flex-col gap-1 rounded-md border border-border/60 bg-card px-3 py-2 text-sm">
        <span className="text-[11px] font-mono uppercase tracking-wide text-muted-foreground">
          tool result
        </span>
        <div className="whitespace-pre-wrap">{message.content}</div>
        {message.meta_json && (
          <details className="text-xs text-muted-foreground">
            <summary className="cursor-pointer select-none">payload</summary>
            <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted/40 p-2 text-[11px]">
              {prettyJson(message.meta_json)}
            </pre>
          </details>
        )}
      </div>
    </div>
  );
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}
