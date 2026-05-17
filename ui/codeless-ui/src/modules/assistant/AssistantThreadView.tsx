import { useCallback, useEffect, useRef, useState } from "react";
import {
  useEventStream,
  useRpc,
  type AssistantAction,
  type AssistantActionCard,
  type AssistantActionStatus,
  type AssistantMessage,
  type AssistantThread,
  type EventEnvelope,
  type JobId,
} from "@/lib/rpc";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { navigate } from "@/lib/route";
import { MarkdownBubble } from "../chat";
import { useAssistantFocus } from "./focusStore";

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
  const refreshTick = useAssistantFocus((s) => s.refreshTick);
  const [messages, setMessages] = useState<AssistantMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // Live planner output. The assistant RPC blocks until the turn
  // finishes; without an event subscription the user would stare at
  // "Sending…" for the full latency of a model call. The planner
  // publishes `ai-token` deltas onto the bus keyed on the thread id
  // reused as a synthetic JobId (see assistant_planner.rs), so the
  // same SSE channel that powers job chats also feeds this view. The
  // streaming buffer is cleared the moment the awaited result lands
  // — at that point the persisted messages are authoritative.
  const [streamingText, setStreamingText] = useState("");
  const [streamingActive, setStreamingActive] = useState(false);
  const scrollAnchorRef = useRef<HTMLDivElement | null>(null);

  // Reload when the parent rail swaps in a different thread, *or*
  // when `refreshTick` bumps — the footer composer increments the
  // tick after a successful `append_assistant_message`, so a
  // message sent from the footer surfaces in this view on the next
  // render without a per-thread subscription channel.
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
  }, [rpc, thread.id, refreshTick]);

  // Scroll to the bottom on new messages — matches every other chat
  // surface and keeps the latest turn in view without the user having
  // to drag the scrollbar.
  useEffect(() => {
    scrollAnchorRef.current?.scrollIntoView({ block: "end" });
  }, [messages.length, streamingText]);

  // Reset the streaming buffer when the parent rail swaps threads so
  // tokens from a prior turn don't bleed into the new transcript.
  useEffect(() => {
    setStreamingText("");
    setStreamingActive(false);
  }, [thread.id]);

  const onAssistantEvent = useCallback(
    (env: EventEnvelope) => {
      const ev = env.event;
      if (ev.type === "ai-token") {
        setStreamingText((prev) => prev + ev.delta);
        setStreamingActive(true);
      } else if (ev.type === "ai-message-complete") {
        // Completion handshake — the awaited RPC result will arrive
        // imminently with the persisted final message; freezing the
        // pulse here avoids a flicker between the last token and the
        // bubble being replaced by the real row.
        setStreamingActive(false);
      }
    },
    [],
  );
  // `since: 0` replays the full thread history on subscribe; the
  // accumulator only renders while `sending` is true so a replay of
  // an old turn doesn't surface as a phantom bubble.
  useEventStream(
    { scope: "job", job_id: thread.id as unknown as JobId },
    onAssistantEvent,
  );

  const onSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      const content = input.trim();
      if (!content || sending) return;
      setSending(true);
      setStreamingText("");
      setStreamingActive(false);
      setErr(null);
      try {
        const res = await rpc.call("append_assistant_message", {
          thread_id: thread.id,
          content,
        });
        // The planner may emit one or more action-card rows alongside
        // the prose reply; they arrive in created_at order, so a plain
        // concatenation matches what a re-list would render. Empty for
        // a plain Q&A turn — slash commands route through the same
        // shape with `assistant_message` carrying the only card.
        setMessages((prev) => [
          ...prev,
          res.user_message,
          res.assistant_message,
          ...(res.cards ?? []),
        ]);
        setInput("");
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setSending(false);
        // Drop the in-flight buffer — on success the persisted
        // assistant message just rendered through `messages`; on
        // failure the partial text would otherwise stick around
        // alongside the error banner with no way to dismiss it.
        setStreamingText("");
        setStreamingActive(false);
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
          {sending && streamingText.length > 0 && (
            <MarkdownBubble
              role="assistant"
              content={streamingText}
              streaming={streamingActive}
            />
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
  // Plain prose turn. Routed through the shared MarkdownBubble so the
  // assistant transcript renders the same markdown surface area as the
  // job chat instead of dumping raw asterisks and fences as text.
  return (
    <MarkdownBubble
      role={message.role === "user" ? "user" : "assistant"}
      content={message.content}
    />
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
        {card.action.tool === "draft_job" && (
          <DraftJobPreview action={card.action} />
        )}
        {card.action.tool === "edit_scope" && (
          <EditScopePreview action={card.action} />
        )}
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

// Structured preview for the `draft_job` action card. The card's
// `content` already carries a human summary, but the draft-review card
// is the one place the user is committing to a multi-field mutation —
// rendering the proposed fields as a table makes the review honest.
// Optional fields (`workspace_mode`, `model`, …) are omitted when
// `null` so the table reflects exactly what `submit_job` will see.
function DraftJobPreview({
  action,
}: {
  action: Extract<AssistantAction, { tool: "draft_job" }>;
}) {
  const rows: Array<[string, string]> = [
    ["repo", action.repo_id],
    ["runner", action.runner],
    ["branch", action.branch],
    ["cost cap", `${action.cost_cap_cents}¢`],
    ["wall clock cap", `${action.wall_clock_cap_ms}ms`],
  ];
  if (action.workspace_mode) rows.push(["workspace", action.workspace_mode]);
  if (action.model) rows.push(["model", action.model]);
  if (action.permission_mode) rows.push(["permission", action.permission_mode]);
  if (action.effort) rows.push(["effort", action.effort]);
  return (
    <div className="mt-1 flex flex-col gap-1 rounded border border-border/40 bg-background/40 p-2 text-xs">
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="font-mono text-muted-foreground">{k}</dt>
            <dd className="truncate font-mono">{v}</dd>
          </div>
        ))}
      </dl>
      <details className="text-muted-foreground">
        <summary className="cursor-pointer select-none">prompt</summary>
        <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted/40 p-2">
          {action.prompt}
        </pre>
      </details>
    </div>
  );
}

// Short `tool:method_name` label for the card header so the user can
// see what they are about to run without parsing the human summary.
function actionLabel(action: AssistantAction): string {
  return `tool:${action.tool}`;
}

// Structured preview for the `edit_scope` action card. Fetches the
// current on-disk file via `read_job_file` so the user can review the
// unified diff (computed in the browser to avoid round-tripping the
// proposed body twice) before confirming the rewrite. The diff is
// presentation-only — the server recomputes its own diff when it
// emits the trailing `Tool` message, which the user trusts because
// the server is the one that actually wrote the file.
function EditScopePreview({
  action,
}: {
  action: Extract<AssistantAction, { tool: "edit_scope" }>;
}) {
  const rpc = useRpc();
  const [current, setCurrent] = useState<string | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setCurrent(null);
    setLoadErr(null);
    void rpc
      .call("read_job_file", {
        job_id: action.job_id,
        filename: action.filename,
      })
      .then((res) => {
        if (cancelled) return;
        setCurrent(res.content);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // NotFound is expected for first-time writes; render as an
        // empty current body so the diff shows every line as an
        // addition, mirroring how the server treats it.
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.toLowerCase().includes("not found")) {
          setCurrent("");
        } else {
          setLoadErr(msg);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, action.job_id, action.filename]);

  const diffLines =
    current === null
      ? null
      : unifiedDiffLines(current, action.new_content);

  return (
    <div className="mt-1 flex flex-col gap-2 rounded border border-border/40 bg-background/40 p-2 text-xs">
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        <dt className="font-mono text-muted-foreground">job</dt>
        <dd className="truncate font-mono">{action.job_id}</dd>
        <dt className="font-mono text-muted-foreground">file</dt>
        <dd className="truncate font-mono">{action.filename}</dd>
        <dt className="font-mono text-muted-foreground">new size</dt>
        <dd className="font-mono">{action.new_content.length} bytes</dd>
      </dl>
      <div className="flex items-center justify-end">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => navigate(`/jobs/${action.job_id}`)}
          aria-label="Open in editor"
        >
          Open in editor
        </Button>
      </div>
      {loadErr ? (
        <div className="text-destructive">
          Could not read current file: {loadErr}
        </div>
      ) : diffLines === null ? (
        <div className="text-muted-foreground">Loading current file…</div>
      ) : (
        <details open className="text-muted-foreground">
          <summary className="cursor-pointer select-none">unified diff</summary>
          <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted/40 p-2 font-mono text-[11px] leading-tight">
            {diffLines.map((l, i) => (
              <div
                key={i}
                className={cn(
                  l.kind === "add" && "text-emerald-500",
                  l.kind === "del" && "text-destructive",
                  l.kind === "header" && "font-semibold text-muted-foreground",
                )}
              >
                {l.text}
              </div>
            ))}
          </pre>
        </details>
      )}
    </div>
  );
}

type DiffLine = { kind: "add" | "del" | "eq" | "header"; text: string };

// Browser-side LCS unified diff. Mirrors the Rust `unified_diff` so the
// preview matches what the server emits on confirm; the runtime
// recomputes its own diff for the `Tool` message rather than trusting
// this one. Kept inside this file because nothing else needs it —
// promoting to `@/lib/diff` would be premature.
function unifiedDiffLines(oldText: string, newText: string): DiffLine[] {
  const a = splitLines(oldText);
  const b = splitLines(newText);
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: DiffLine[] = [
    { kind: "header", text: "--- current" },
    { kind: "header", text: "+++ proposed" },
  ];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "eq", text: " " + a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ kind: "del", text: "-" + a[i] });
      i++;
    } else {
      out.push({ kind: "add", text: "+" + b[j] });
      j++;
    }
  }
  while (i < n) {
    out.push({ kind: "del", text: "-" + a[i++] });
  }
  while (j < m) {
    out.push({ kind: "add", text: "+" + b[j++] });
  }
  return out;
}

function splitLines(s: string): string[] {
  if (s.length === 0) return [];
  const lines = s.split("\n");
  // `split("\n")` leaves a trailing empty string when the source ends
  // with a newline; drop it so the diff doesn't show a phantom blank
  // addition at the end of every file.
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
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
