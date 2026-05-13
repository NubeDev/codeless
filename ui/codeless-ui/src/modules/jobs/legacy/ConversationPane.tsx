// Unified, chronological conversation view for a single job — the
// centre column of the JOBS-UX redesign (see DOCS/JOBS-UX.md). One
// scroll-pane contains every observation a user has about the job:
// the agent's streaming reasoning, tool calls (collapsed cards),
// stage transitions (dividers), errors, and lifecycle moments. The
// composer (Phase 3) lives below it; the right pane (Phase 4) hosts
// ambient signal (status, cost, runtime, drill-down tabs).
//
// Today: read-only. The composer + state-driven controls land in
// Phase 3, gated on A0 (intra-stage session continuation) so
// pause / resume actually do what the buttons promise. The existing
// `JobChat` in `RunPane` continues to render in its own pane until
// then.

import { useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";
import { useEventStream, type Job, type JobId } from "@/lib/rpc";
import type { EventEnvelope } from "@/lib/rpc/wire";
import { Streamdown } from "streamdown";

import { Composer } from "./Composer";

interface Props {
  jobId: JobId;
  // Live job row, owned by the JobPage's useJob() hook. The composer
  // reads status/caps/stop_reason off it to render the right
  // primary action; `null` when the parent hasn't resolved the job
  // yet (e.g. initial load), in which case the composer is hidden.
  job: Job | null;
  refetchJob: () => void;
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
  // Optional override: when the parent has already paid for a
  // `?since=0` replay for the same job, the pane can join that
  // existing shared subscription rather than opening a parallel one.
  // Defaults to 0 (full replay), which is the right call for a tab
  // the user just opened.
  since?: number;
}

// Messages the pane renders are not 1:1 with bus events — multiple
// `ai-token` deltas fold into one streaming `agent_msg`; one
// `tool-call` event becomes one `tool_call` message; lifecycle
// events become `lifecycle` dividers. The fold is computed
// incrementally as events arrive so there's no full re-traversal
// per token.
type Message =
  | {
      kind: "agent_msg";
      // task_id is the grouping key: one streaming bubble per task,
      // closed by the matching `ai-message-complete` event.
      task_id: string;
      ts: number;
      text: string;
      streaming: boolean;
      // Populated on completion.
      input_tokens?: number;
      output_tokens?: number;
      cost_cents?: number;
    }
  | {
      kind: "tool_call";
      cursor: number;
      ts: number;
      tool: string;
      args_json: string;
    }
  | {
      kind: "lifecycle";
      cursor: number;
      ts: number;
      label: string;
      tone: "neutral" | "good" | "bad" | "warn";
    }
  | {
      kind: "error";
      cursor: number;
      ts: number;
      text: string;
    };

// Ordered list with stable identity so React's keying works. Each
// streaming bubble is keyed by `task_id`; lifecycle / tool_call /
// error are keyed by cursor (always unique server-side).
function messageKey(m: Message): string {
  if (m.kind === "agent_msg") return `agent:${m.task_id}`;
  return `${m.kind}:${m.cursor}`;
}

export function ConversationPane({
  jobId,
  job,
  refetchJob,
  onOpenJobTab,
  since = 0,
}: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [showRawEvents, setShowRawEvents] = useState(false);
  const [rawEvents, setRawEvents] = useState<EventEnvelope[]>([]);

  // Auto-scroll: keep the bottom in view while the user is at (or
  // near) the bottom; do nothing while the user has scrolled up to
  // read history. The "near the bottom" tolerance keeps the
  // behaviour intuitive when a new line slightly lifts the scroll
  // position.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickToBottomRef = useRef(true);
  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const distFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    stickToBottomRef.current = distFromBottom < 80;
  };
  useEffect(() => {
    if (!stickToBottomRef.current) return;
    const el = scrollRef.current;
    if (!el) return;
    // Defer to next frame so the layout has settled.
    requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
    });
  }, [messages, showRawEvents]);

  useEventStream(
    { scope: "job", job_id: jobId },
    (env) => {
      setRawEvents((prev) => [...prev, env]);
      setMessages((prev) => foldEvent(prev, env));
    },
    since,
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ConversationHeader
        showRaw={showRawEvents}
        onToggleRaw={() => setShowRawEvents((s) => !s)}
        eventCount={rawEvents.length}
      />
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div className="mx-auto max-w-[820px] px-4 py-3">
          {showRawEvents ? (
            <RawEventsView events={rawEvents} />
          ) : (
            <ConversationList messages={messages} />
          )}
        </div>
      </div>
      {job && (
        <Composer
          job={job}
          refetchJob={refetchJob}
          onOpenJobTab={onOpenJobTab}
        />
      )}
    </div>
  );
}

function ConversationHeader({
  showRaw,
  onToggleRaw,
  eventCount,
}: {
  showRaw: boolean;
  onToggleRaw: () => void;
  eventCount: number;
}) {
  return (
    <div className="border-border/50 flex shrink-0 items-center justify-between gap-3 border-b px-4 py-1.5">
      <span className="text-muted-foreground text-[10px] uppercase tracking-wide">
        Conversation
      </span>
      <button
        type="button"
        onClick={onToggleRaw}
        className={cn(
          "text-muted-foreground hover:text-foreground text-[10px] uppercase tracking-wide",
        )}
        title="Switch between the conversation view (default) and the raw event log (debugging)."
      >
        {showRaw ? `← conversation` : `raw events (${eventCount}) →`}
      </button>
    </div>
  );
}

function ConversationList({ messages }: { messages: Message[] }) {
  if (messages.length === 0) {
    return (
      <div className="text-muted-foreground py-8 text-center text-xs">
        Waiting for the first event. Submit the job to start the run.
      </div>
    );
  }
  return (
    <ul className="space-y-2">
      {messages.map((m) => (
        <MessageRow key={messageKey(m)} message={m} />
      ))}
    </ul>
  );
}

function MessageRow({ message }: { message: Message }) {
  switch (message.kind) {
    case "agent_msg":
      return <AgentBubble message={message} />;
    case "tool_call":
      return (
        <ToolCard
          tool={message.tool}
          argsJson={message.args_json}
          ts={message.ts}
        />
      );
    case "lifecycle":
      return (
        <LifecycleDivider label={message.label} tone={message.tone} ts={message.ts} />
      );
    case "error":
      return <ErrorCard text={message.text} ts={message.ts} />;
  }
}

function AgentBubble({
  message,
}: {
  message: Extract<Message, { kind: "agent_msg" }>;
}) {
  return (
    <li className="border-blue-500/30 bg-blue-500/5 rounded-md border px-2.5 py-2">
      <div className="text-muted-foreground mb-1 flex items-center justify-between gap-2 text-[9px] uppercase tracking-wide">
        <span className="text-blue-700 dark:text-blue-300">assistant</span>
        {message.streaming ? (
          <span
            className="bg-blue-500 inline-block h-1.5 w-1.5 animate-pulse rounded-full"
            title="streaming"
          />
        ) : (
          <span className="font-mono normal-case tracking-normal">
            {shortTime(message.ts)}
            {typeof message.cost_cents === "number" &&
              message.cost_cents > 0 && (
                <span className="ml-2">${(message.cost_cents / 100).toFixed(2)}</span>
              )}
          </span>
        )}
      </div>
      <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] break-words [&_pre]:my-1.5 [&_pre]:bg-background/60 [&_pre]:p-2 [&_pre]:text-[11px] [&_pre]:whitespace-pre-wrap [&_pre]:break-words [&_pre]:overflow-x-auto [&_code]:bg-background/60 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[11px] [&_code]:break-all [&_h1]:text-sm [&_h2]:text-sm [&_h3]:text-[13px] [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_p]:my-1 [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
        <Streamdown>{message.text}</Streamdown>
      </div>
    </li>
  );
}

function ToolCard({
  tool,
  argsJson,
  ts,
}: {
  tool: string;
  argsJson: string;
  ts: number;
}) {
  const [expanded, setExpanded] = useState(false);
  const summary = useMemo(() => oneLineToolSummary(tool, argsJson), [tool, argsJson]);
  return (
    <li>
      <button
        type="button"
        onClick={() => setExpanded((e) => !e)}
        className={cn(
          "border-border/40 bg-muted/20 hover:bg-muted/40 flex w-full items-center justify-between gap-2 rounded border px-2 py-1.5 text-left text-[11px] transition-colors",
        )}
      >
        <span className="min-w-0 flex-1 truncate">
          <span className="text-muted-foreground mr-1.5">tool</span>
          <span className="font-mono font-medium">{tool}</span>
          {summary && (
            <span className="text-muted-foreground ml-2 truncate">{summary}</span>
          )}
        </span>
        <span className="text-muted-foreground shrink-0 font-mono text-[10px]">
          {shortTime(ts)}
        </span>
      </button>
      {expanded && (
        <pre className="border-border/40 bg-background/60 mt-1 max-h-72 overflow-auto rounded border px-2 py-1.5 font-mono text-[10px] whitespace-pre-wrap break-all">
          {prettyJson(argsJson)}
        </pre>
      )}
    </li>
  );
}

function LifecycleDivider({
  label,
  tone,
  ts,
}: {
  label: string;
  tone: "neutral" | "good" | "bad" | "warn";
  ts: number;
}) {
  const colour =
    tone === "good"
      ? "text-emerald-600 dark:text-emerald-400 border-emerald-500/30"
      : tone === "bad"
        ? "text-destructive border-destructive/30"
        : tone === "warn"
          ? "text-amber-600 dark:text-amber-400 border-amber-500/40"
          : "text-muted-foreground border-border/50";
  return (
    <li
      className={cn(
        "flex items-center gap-2 py-1 font-mono text-[10px] uppercase tracking-wide",
        colour,
      )}
    >
      <span className={cn("h-px flex-1 border-t", colour)} />
      <span>{label}</span>
      <span className="text-muted-foreground normal-case tracking-normal">
        {shortTime(ts)}
      </span>
      <span className={cn("h-px flex-1 border-t", colour)} />
    </li>
  );
}

function ErrorCard({ text, ts }: { text: string; ts: number }) {
  return (
    <li className="border-destructive/40 bg-destructive/5 rounded border px-2.5 py-1.5 text-xs">
      <div className="text-destructive mb-0.5 flex items-center justify-between text-[9px] uppercase tracking-wide">
        <span>error</span>
        <span className="text-muted-foreground font-mono normal-case tracking-normal">
          {shortTime(ts)}
        </span>
      </div>
      <div>{text}</div>
    </li>
  );
}

function RawEventsView({ events }: { events: EventEnvelope[] }) {
  if (events.length === 0) {
    return (
      <div className="text-muted-foreground py-8 text-center text-xs">
        No events yet.
      </div>
    );
  }
  return (
    <ul className="space-y-0.5">
      {events.map((env) => (
        <li
          key={env.cursor}
          className="border-border/30 border-l pl-2 font-mono text-[10px]"
        >
          <span className="text-muted-foreground mr-2">
            {shortTime(env.created_at)}
          </span>
          <span className="font-medium">{env.event.type}</span>
        </li>
      ))}
    </ul>
  );
}

// Incrementally fold one event envelope into the running message
// list. The fold is pure (no side effects) so React state updates
// stay predictable, and it's incremental so a streaming `ai-token`
// flurry doesn't re-render the whole history per token.
function foldEvent(prev: Message[], env: EventEnvelope): Message[] {
  const e = env.event;
  const ts = env.created_at;
  switch (e.type) {
    case "ai-token": {
      const taskId = e.task_id;
      // Append into the open agent bubble for this task, or open a
      // new one. Find from the tail backwards — streaming bubbles
      // are typically the last few rows; a full prev.length scan
      // would be wasteful as the conversation grows.
      for (let i = prev.length - 1; i >= 0; i--) {
        const m = prev[i];
        if (m.kind === "agent_msg" && m.task_id === taskId) {
          if (!m.streaming) break;
          const next = prev.slice();
          next[i] = { ...m, text: m.text + e.delta };
          return next;
        }
      }
      return [
        ...prev,
        {
          kind: "agent_msg",
          task_id: taskId,
          ts,
          text: e.delta,
          streaming: true,
        },
      ];
    }
    case "ai-message-complete": {
      const taskId = e.task_id;
      for (let i = prev.length - 1; i >= 0; i--) {
        const m = prev[i];
        if (m.kind === "agent_msg" && m.task_id === taskId && m.streaming) {
          const next = prev.slice();
          next[i] = {
            ...m,
            streaming: false,
            input_tokens: e.input_tokens,
            output_tokens: e.output_tokens,
            cost_cents: e.cost_cents,
          };
          return next;
        }
      }
      return prev;
    }
    case "tool-call":
    case "tool-approval-requested":
      return [
        ...prev,
        {
          kind: "tool_call",
          cursor: env.cursor,
          ts,
          tool: e.tool,
          args_json: e.args_json,
        },
      ];

    case "stage-started": {
      // `ordinal` is `Option<u32>` on the wire — older event rows
      // (pre-the-ordinal-on-wire change) carry None.
      const label =
        typeof e.ordinal === "number"
          ? `stage ${e.ordinal + 1} started: ${e.name}`
          : `stage started: ${e.name}`;
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label,
          tone: "neutral",
        },
      ];
    }
    case "stage-completed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: `stage ${e.status}`,
          tone: e.status === "passed" ? "good" : "bad",
        },
      ];
    case "verify-started":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "verify started",
          tone: "neutral",
        },
      ];
    case "verify-passed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "verify passed",
          tone: "good",
        },
      ];
    case "verify-failed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: `verify failed (exit ${e.exit_code})`,
          tone: "bad",
        },
      ];
    case "job-queued":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "queued",
          tone: "neutral",
        },
      ];
    case "job-promoted":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "promoted",
          tone: "neutral",
        },
      ];
    case "job-started":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "job started",
          tone: "good",
        },
      ];
    case "job-completed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "job completed",
          tone: "good",
        },
      ];
    case "job-stopped":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: `stopped: ${e.reason}`,
          tone: e.reason === "user" ? "neutral" : "warn",
        },
      ];
    case "job-failed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "job failed",
          tone: "bad",
        },
      ];
    case "job-resumed":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: e.previous_reason
            ? `resumed (was ${e.previous_reason})`
            : "resumed",
          tone: "good",
        },
      ];
    case "review-requested":
      return [
        ...prev,
        {
          kind: "lifecycle",
          cursor: env.cursor,
          ts,
          label: "review requested",
          tone: "warn",
        },
      ];
    case "stage-session-captured":
      // Observability-only event — the conversation already shows
      // the agent's output that produced the session id. Folding
      // every captured-session-id event into a dividing line would
      // be noise; expose it via the raw-events toggle only.
      return prev;
    default:
      return prev;
  }
}

// "Read foo.rs" / "Edit bar.rs" / "Bash <first 40 chars>" — enough
// for the collapsed card to be useful at a glance. Falls back to
// the empty string when the args don't have a recognisable shape;
// the card still renders, just without the trailing summary.
function oneLineToolSummary(tool: string, argsJson: string): string {
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return "";
  }
  if (parsed == null || typeof parsed !== "object") return "";
  const obj = parsed as Record<string, unknown>;
  const path =
    (typeof obj.file_path === "string" && obj.file_path) ||
    (typeof obj.path === "string" && obj.path) ||
    "";
  if (path) return shortPath(path);
  if (tool.toLowerCase() === "bash" && typeof obj.command === "string") {
    const cmd = obj.command.trim().split("\n")[0];
    return cmd.length > 60 ? `${cmd.slice(0, 60)}…` : cmd;
  }
  if (typeof obj.pattern === "string") return obj.pattern;
  if (typeof obj.query === "string") return obj.query;
  return "";
}

function shortPath(p: string): string {
  // Trim leading worktree path so the summary doesn't waste width on
  // `/tmp/codeless-worktrees/job-…/crates/codeless-types/src/foo.rs`.
  // Keep the last two path segments so `mod.rs` files stay
  // disambiguated by their parent dir.
  const idx = p.lastIndexOf("/");
  if (idx < 0) return p;
  const parentIdx = p.lastIndexOf("/", idx - 1);
  if (parentIdx < 0) return p.slice(idx + 1);
  return `…${p.slice(parentIdx)}`;
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

function shortTime(ms: number): string {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}
