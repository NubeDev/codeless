import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { useRpc, type AssistantMessage } from "@/lib/rpc";
import { useChatStore } from "@/modules/ai";
import { useAssistantFocus } from "./focusStore";

// Footer composer driven by the assistant. Replaces the in-editor
// AI panel's `AiInputBar` as the primary chat entry point: every tab
// has the assistant one keystroke away without switching to
// `/assistant`. Submissions persist to SQLite via
// `append_assistant_message`, so a re-render of the `/assistant`
// transcript shows them (no footer-local buffer — see SCOPE.md F1).
//
// Action cards rendered full-width (draft review, scope diff) stay on
// the `/assistant` page. The footer surfaces a compact
// "open in /assistant to confirm" affordance instead — the composer
// is for input + short responses, not full-screen review.
export type AssistantFooterBarProps = {
  // Brings the `/assistant` tab to the front. The bar invokes this
  // from the "Open in /assistant" affordance and from the "Switch to
  // thread" hint when the rail is on a different surface. The footer
  // does not own routing — App.tsx wires this to `newAssistantTab`.
  onOpenAssistant: () => void;
};

export function AssistantFooterBar({ onOpenAssistant }: AssistantFooterBarProps) {
  const rpc = useRpc();
  const currentThreadId = useAssistantFocus((s) => s.currentThreadId);
  const setCurrentThreadId = useAssistantFocus((s) => s.setCurrentThreadId);
  const bumpRefresh = useAssistantFocus((s) => s.bumpRefresh);
  const refreshTick = useAssistantFocus((s) => s.refreshTick);

  const [value, setValue] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [pendingCards, setPendingCards] = useState<number>(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // `useChatStore.focusSignal` is the legacy "focus the AI composer"
  // bus — `Ctrl-K` / "Ask AI" use it. The store has lost message
  // ownership in F1 but its UI-presentation slots (focus signal,
  // panel-open flag, composer draft prefill) live on; the footer
  // subscribes here so the same keyboard shortcut still drops the
  // caret in the textarea.
  const focusSignal = useChatStore((s) => s.focusSignal);
  useEffect(() => {
    if (focusSignal === 0) return;
    textareaRef.current?.focus();
  }, [focusSignal]);

  // Auto-grow the textarea up to a sane cap so a multi-line draft is
  // visible without colonising the workspace area. Mirrors the cap in
  // the in-editor AI composer for muscle-memory parity.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [value]);

  // Probe the current thread for pending action cards. The planner
  // emits cards as ordinary assistant-role messages with `meta_json`
  // carrying `kind == "action_card"` + `status`; we count the ones
  // still awaiting confirm/cancel so the footer can offer a "open in
  // /assistant to confirm" affordance without rendering the cards
  // themselves (those live full-width on `/assistant`).
  useEffect(() => {
    if (!currentThreadId) {
      setPendingCards(0);
      return;
    }
    let cancelled = false;
    void rpc
      .call("list_assistant_messages", { thread_id: currentThreadId })
      .then((res) => {
        if (cancelled) return;
        setPendingCards(countPendingCards(res.messages));
      })
      .catch(() => {
        if (cancelled) return;
        // A stale id (thread deleted on the server) shows up as an
        // error here. Clear the focus so the next submission either
        // creates a fresh thread or picks one the user resolves via
        // the rail; either way the footer becomes useful again.
        setPendingCards(0);
        setCurrentThreadId(null);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, currentThreadId, refreshTick, setCurrentThreadId]);

  const onNewThread = useCallback(async () => {
    if (sending) return;
    setSending(true);
    setErr(null);
    try {
      // PS5: persona is required at creation; the footer bar uses
      // the seeded general persona until the UI grows a picker.
      const created = await rpc.call("create_assistant_thread", {
        persona_id: "builtin:general",
      });
      setCurrentThreadId(created.id);
      bumpRefresh();
      textareaRef.current?.focus();
    } catch (e: unknown) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }, [rpc, sending, setCurrentThreadId, bumpRefresh]);

  const onSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      const content = value.trim();
      if (!content || sending) return;
      setSending(true);
      setErr(null);
      try {
        let threadId = currentThreadId;
        if (!threadId) {
          // Lazy thread creation on first submit keeps the empty
          // state honest — the user doesn't have to click "New" before
          // saying hello. The created id is persisted so the next
          // launch keeps the same conversation focused.
          const created = await rpc.call("create_assistant_thread", {
            persona_id: "builtin:general",
          });
          threadId = created.id;
          setCurrentThreadId(threadId);
        }
        await rpc.call("append_assistant_message", {
          thread_id: threadId,
          content,
        });
        setValue("");
        bumpRefresh();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setSending(false);
      }
    },
    [rpc, value, sending, currentThreadId, setCurrentThreadId, bumpRefresh],
  );

  const canSend = !sending && value.trim().length > 0;
  const placeholder = useMemo(
    () =>
      currentThreadId
        ? "Message the assistant — Enter to send, Shift+Enter for newline"
        : "Ask the assistant — your first message starts a new thread",
    [currentThreadId],
  );

  return (
    <div
      data-ai-input-bar
      className="shrink-0 border-t border-border/60 bg-card/40 px-3 py-2"
    >
      <form
        onSubmit={onSubmit}
        className="flex flex-col gap-1.5 rounded-lg px-1 py-1"
      >
        <div className="flex items-start gap-2">
          <textarea
            ref={textareaRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void onSubmit();
              }
            }}
            placeholder={placeholder}
            rows={1}
            disabled={sending}
            className={cn(
              "max-h-40 flex-1 resize-none bg-transparent text-[13px] leading-relaxed outline-none",
              "placeholder:text-muted-foreground/60",
            )}
          />
          <div className="flex shrink-0 items-center gap-1">
            <Button
              type="button"
              size="xs"
              variant="ghost"
              onClick={() => void onNewThread()}
              disabled={sending}
              aria-label="Start a new assistant thread"
              title="Start a new assistant thread"
            >
              New thread
            </Button>
            <Button
              type="submit"
              size="xs"
              disabled={!canSend}
              aria-label="Send message"
            >
              {sending ? <Spinner className="size-3" /> : "Send"}
            </Button>
          </div>
        </div>

        {pendingCards > 0 && currentThreadId ? (
          <button
            type="button"
            onClick={onOpenAssistant}
            className="flex items-center justify-between gap-2 rounded-md border border-yellow-500/40 bg-yellow-500/5 px-2 py-1 text-left text-[11px] text-foreground transition-colors hover:bg-yellow-500/10"
          >
            <span>
              {pendingCards} pending action
              {pendingCards === 1 ? "" : "s"} — review and confirm in{" "}
              <span className="font-mono">/assistant</span>
            </span>
            <span aria-hidden className="text-muted-foreground">
              →
            </span>
          </button>
        ) : null}

        {err ? (
          <div className="rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
            {err}
          </div>
        ) : null}
      </form>
    </div>
  );
}

// `meta_json` carries an `AssistantActionCard`; only the `status`
// field decides "still actionable" so we parse defensively and skip
// anything that does not look like a card. Matches the renderer in
// `AssistantThreadView`.
function countPendingCards(messages: AssistantMessage[]): number {
  let n = 0;
  for (const m of messages) {
    if (m.role !== "assistant" || !m.meta_json) continue;
    try {
      const parsed = JSON.parse(m.meta_json) as {
        kind?: string;
        status?: string;
      };
      if (parsed.kind === "action_card" && parsed.status === "pending") {
        n++;
      }
    } catch {
      // ignore non-JSON meta — falls back to the plain-message case.
    }
  }
  return n;
}
