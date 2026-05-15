import { useCallback, useEffect, useRef, useState } from "react";
import { useRpc, type AssistantMessage, type AssistantThread } from "@/lib/rpc";
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
            messages.map((m) => <MessageBubble key={m.id} message={m} />)
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

function MessageBubble({ message }: { message: AssistantMessage }) {
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
