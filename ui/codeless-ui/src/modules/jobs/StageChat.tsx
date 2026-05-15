import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  useEventStream,
  useJob,
  useRpc,
  type EventEnvelope,
  type JobId,
} from "@/lib/rpc";

// ------------------------------------------------------------------ types

interface ChatMessage {
  role: "user" | "assistant";
  text: string;
  ts: string;
}

// A divider row inserted when session-archived-then-resumed fires.
// Rendered inline between turns so the user sees exactly where the
// session boundary fell without any extra navigation.
interface ArchivedDivider {
  kind: "archived";
  key: string;
}

type FeedRow =
  | { kind: "message"; message: ChatMessage }
  | ArchivedDivider;

// ------------------------------------------------------------------ helpers

function isoNow(): string {
  return new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
}

function shortTime(iso: string): string {
  return iso.replace("T", " ").replace("Z", "");
}

// Map the job runner id to the CLI runner id agent_chat accepts.
// REST runners (anthropic, openai) fall back to claude because
// agent_chat only accepts CLI runner ids.
function cliRunnerFor(jobRunner: string): string {
  if (jobRunner === "claude") return "claude";
  if (jobRunner === "copilot") return "copilot";
  if (jobRunner === "codex") return "codex";
  return "claude";
}

// Build the prompt for a stage chat turn. Includes enough context
// about the stage so the agent knows what it's being asked about
// without requiring --continue to be supported. Prior transcript
// is included so multi-turn context is preserved across one-shot
// agent_chat calls (which are stateless per v1).
function buildStageChatPrompt(
  history: ChatMessage[],
  stageName: string,
): string {
  const header = `You are answering questions about a failed or completed CI stage named "${stageName}". The stage ran inside a git worktree. Answer questions about the code, errors, or output from that stage.`;

  if (history.length === 1) {
    return `${header}\n\n${history[0].text}`;
  }

  const lines: string[] = [header, "", "Prior turns in this conversation follow.", ""];
  for (const m of history.slice(0, -1)) {
    lines.push(`### ${m.role === "user" ? "User" : "Assistant"}`);
    lines.push(m.text);
    lines.push("");
  }
  lines.push("### User");
  lines.push(history[history.length - 1].text);
  lines.push("");
  lines.push("Reply directly to the latest user message.");
  return lines.join("\n");
}

// Wait for ai-message-complete on the given task, accumulating ai-token
// deltas into the returned text. Opens its own subscription so the
// result is a single awaitable Promise; the shared-subscription layer
// keeps the underlying SSE connection count to one.
function waitForCompletion(
  rpc: ReturnType<typeof useRpc>,
  sessionId: JobId,
  taskId: string,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let text = "";
    let done = false;
    const stream = rpc.subscribe({ scope: "job", job_id: sessionId });
    const iter = stream[Symbol.asyncIterator]();
    (async () => {
      try {
        while (!done) {
          const r = await iter.next();
          if (r.done) {
            if (!done) reject(new Error("event stream closed before completion"));
            return;
          }
          const env = r.value;
          if (env.task_id !== taskId) continue;
          const e = env.event;
          if (e.type === "ai-token") {
            text += e.delta;
          } else if (e.type === "ai-message-complete") {
            done = true;
            iter.return?.();
            resolve(text);
            return;
          }
        }
      } catch (e) {
        if (!done) reject(e instanceof Error ? e : new Error(String(e)));
      }
    })();
  });
}

// Round-trips renderChatMarkdown's `## role @ ts` headings.
function parseChatMarkdown(src: string): ChatMessage[] {
  const out: ChatMessage[] = [];
  const lines = src.split("\n");
  let current: ChatMessage | null = null;
  let buf: string[] = [];
  const flush = () => {
    if (current) {
      current.text = buf.join("\n").trim();
      if (current.text) out.push(current);
    }
    current = null;
    buf = [];
  };
  for (const raw of lines) {
    const m = /^##\s+(user|assistant)\s+@\s+(.+)$/i.exec(raw.trim());
    if (m) {
      flush();
      current = {
        role: m[1].toLowerCase() === "user" ? "user" : "assistant",
        ts: m[2].trim(),
        text: "",
      };
      continue;
    }
    if (current) buf.push(raw);
  }
  flush();
  return out;
}

function renderChatMarkdown(messages: ChatMessage[]): string {
  const out: string[] = [`# Stage chat`, ""];
  for (const m of messages) {
    out.push(`## ${m.role} @ ${m.ts}`);
    out.push("");
    out.push(m.text);
    out.push("");
  }
  return out.join("\n");
}

// ------------------------------------------------------------------ component

interface Props {
  jobId: JobId;
  stageId: string;
  stageName: string;
  // The Claude session_id captured from the last stage run. Included
  // in the prompt preamble as context. A future backend revision that
  // adds `previous_session_id` to AgentChatArgs will enable --continue;
  // until then, the full transcript is included in each call.
  capturedSessionId: string | null;
  // Called when the chat transitions between idle and streaming so the
  // parent can drive tab indicators without subscribing to the chat
  // session's event stream itself.
  onChatActive?: (active: boolean) => void;
}

// Live chat panel for one stage tab. Each turn calls agent_chat with
// session_id = stageId so the events flow through a dedicated SSE
// filter and don't mix with job-level chat traffic. The stage context
// (name, prior session id) is included in the prompt preamble because
// agent_chat is one-shot per v1 — no --continue on the wire yet.
//
// The chat is persisted per-stage via write_job_file so it survives
// tab closes and page refreshes. The filename includes the stageId to
// avoid collisions between stages on the same job.
export function StageChat({
  jobId,
  stageId,
  stageName,
  capturedSessionId: _capturedSessionId,
  onChatActive,
}: Props) {
  const rpc = useRpc();
  // The stageId is a valid caller-minted correlation id. Events emitted
  // by agent_chat flow under { scope: "job", job_id: stageId } on the
  // server because events.job_id has no FK constraint and the filter
  // matches on equality. See AgentChatArgs.session_id wire comment.
  const chatSessionId = stageId as JobId;
  const chatFile = `STAGE-CHAT-${stageId}.md`;

  const { data: job } = useJob(jobId);

  const [history, setHistory] = useState<ChatMessage[]>([]);
  const [feed, setFeed] = useState<FeedRow[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [streaming, setStreaming] = useState<{
    taskId: string;
    text: string;
  } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // Tracks whether the user was warned that stopping the turn may leave
  // the completion promise dangling. Internal only — the UI just clears
  // state and lets the promise resolve harmlessly later.
  const abortedRef = useRef(false);
  const listEndRef = useRef<HTMLLIElement | null>(null);

  // Load prior chat history from the per-stage file on mount.
  useEffect(() => {
    let cancelled = false;
    setHistory([]);
    setFeed([]);
    setLoaded(false);
    rpc
      .call("read_job_file", { job_id: jobId, filename: chatFile })
      .then((r) => {
        if (cancelled) return;
        const msgs = parseChatMarkdown(r.content);
        setHistory(msgs);
        setFeed(msgs.map((m) => ({ kind: "message" as const, message: m })));
        setLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId, stageId, chatFile]);

  // Scroll the feed to the bottom whenever new content arrives so the
  // user sees streaming tokens without manually scrolling.
  useEffect(() => {
    listEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [feed, streaming]);

  // Subscribe to the stage chat event stream. The session id matches the
  // stageId so only this stage's chat events are received here.
  const onChatEvent = useCallback(
    (env: EventEnvelope) => {
      const e = env.event;
      if (e.type === "ai-token" && env.task_id) {
        setStreaming((s) =>
          s && s.taskId === env.task_id
            ? { ...s, text: s.text + e.delta }
            : s,
        );
      }
    },
    [],
  );
  useEventStream({ scope: "job", job_id: chatSessionId }, onChatEvent);

  // Subscribe to the job-level stream to catch session lifecycle events.
  // session-archived-then-resumed is emitted against the job's event
  // bus (not the chat session bus) so it's intercepted here.
  const onJobEvent = useCallback(
    (env: EventEnvelope) => {
      const e = env.event;
      if (e.type === "session-archived-then-resumed" && env.stage_id === stageId) {
        setFeed((prev) => [
          ...prev,
          { kind: "archived", key: `archived-${env.cursor}` },
        ]);
      }
    },
    [stageId],
  );
  useEventStream({ scope: "job", job_id: jobId }, onJobEvent);

  // Notify the parent whenever the streaming state changes. The parent
  // uses this to set the tab indicator without subscribing to the chat
  // session stream itself.
  const isActive = busy || streaming !== null;
  const onChatActiveRef = useRef(onChatActive);
  onChatActiveRef.current = onChatActive;
  useEffect(() => {
    onChatActiveRef.current?.(isActive);
  }, [isActive]);

  const stopTurn = () => {
    const taskId = streaming?.taskId;
    abortedRef.current = true;
    setStreaming(null);
    setBusy(false);
    if (taskId && taskId !== "pending") {
      rpc.call("cancel_chat_task", { task_id: taskId }).catch(() => {
        // Non-fatal: the task may have already completed naturally.
      });
    }
  };

  const send = async () => {
    const text = input.trim();
    if (!text || busy) return;

    abortedRef.current = false;
    setBusy(true);
    setErr(null);

    const ts = isoNow();
    const userMsg: ChatMessage = { role: "user", text, ts };
    const optimistic = [...history, userMsg];
    setHistory(optimistic);
    setFeed((prev) => [...prev, { kind: "message", message: userMsg }]);
    setInput("");
    setStreaming({ taskId: "pending", text: "" });

    try {
      const cwd = job?.worktree_path ?? null;
      const runner = cliRunnerFor(job?.runner ?? "claude");

      const result = await rpc.call("agent_chat", {
        runner,
        prompt: buildStageChatPrompt(optimistic, stageName),
        session_id: chatSessionId,
        cwd,
        context: {
          ui_location: `jobs/${jobId}/stages/${stageId}`,
          attachments: [],
          selection: null,
          user_prompts: [],
          job_refs: [],
        },
        mode: "work",
      });

      if (abortedRef.current) return;
      setStreaming({ taskId: result.task_id, text: "" });

      const assistantText = await waitForCompletion(
        rpc,
        chatSessionId,
        result.task_id,
      );

      if (abortedRef.current) return;

      const assistantMsg: ChatMessage = {
        role: "assistant",
        text: assistantText,
        ts: isoNow(),
      };
      const updated = [...optimistic, assistantMsg];
      setHistory(updated);
      setFeed((prev) => [
        ...prev,
        { kind: "message", message: assistantMsg },
      ]);
      setStreaming(null);

      await rpc.call("write_job_file", {
        job_id: jobId,
        filename: chatFile,
        content: renderChatMarkdown(updated),
      });
    } catch (e) {
      if (!abortedRef.current) {
        setErr(e instanceof Error ? e.message : String(e));
        setStreaming(null);
        // Roll back optimistic user message so the user can retry.
        setHistory(history);
        setFeed((prev) =>
          prev.filter(
            (r) =>
              r.kind !== "message" ||
              r.message.role !== "user" ||
              r.message.ts !== ts,
          ),
        );
      }
    } finally {
      if (!abortedRef.current) setBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Header */}
      <div className="shrink-0 flex items-baseline justify-between gap-2 border-t border-border/40 px-1 pb-1.5 pt-3">
        <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
          chat with this stage
        </div>
        <span className="text-muted-foreground font-mono text-[10px]">
          {loaded
            ? `${history.length} message${history.length === 1 ? "" : "s"}`
            : "loading…"}
        </span>
      </div>

      {/* Message feed */}
      <ul className="min-h-0 flex-1 space-y-1.5 overflow-y-auto px-1 pb-1">
        {loaded && history.length === 0 && !streaming && (
          <li className="text-muted-foreground py-3 text-center text-[11px] italic">
            ask a question about this stage — output, errors, or what to do next
          </li>
        )}
        {feed.map((row, i) => {
          if (row.kind === "archived") {
            return <SessionArchivedDivider key={row.key} />;
          }
          return <StageChatBubble key={`m-${i}`} message={row.message} />;
        })}
        {streaming && (
          <StageChatBubble
            message={{ role: "assistant", text: streaming.text || "…", ts: "" }}
            streaming
          />
        )}
        {/* Invisible anchor for auto-scroll */}
        <li ref={listEndRef} aria-hidden className="h-px" />
      </ul>

      {/* Composer */}
      <div className="shrink-0 space-y-1.5 pt-1">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          rows={2}
          placeholder={`ask about stage "${stageName}"…`}
          className="border-border/60 bg-background w-full resize-none rounded border px-2 py-1.5 text-xs"
          disabled={busy}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            onClick={busy ? stopTurn : () => void send()}
            disabled={busy ? false : !input.trim()}
            className={cn(
              busy
                ? "bg-rose-600 text-white hover:bg-rose-700"
                : "bg-blue-600 text-white hover:bg-blue-700",
            )}
          >
            {busy ? "stop ■" : "send ▶"}
          </Button>
          <span className="text-muted-foreground text-[10px]">
            runs in{" "}
            {job?.worktree_path ? "worktree" : "repo root"}
            {" · ⌘/Ctrl+Enter"}
          </span>
        </div>
        {err && <div className="text-destructive text-xs">{err}</div>}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ sub-components

function StageChatBubble({
  message,
  streaming,
}: {
  message: ChatMessage;
  streaming?: boolean;
}) {
  const isUser = message.role === "user";
  return (
    <li
      className={cn(
        "rounded-md border px-2.5 py-2",
        isUser
          ? "border-zinc-500/30 bg-zinc-500/5"
          : "border-blue-500/30 bg-blue-500/5",
      )}
    >
      <div className="text-muted-foreground mb-1 flex items-center justify-between gap-1.5 text-[9px] uppercase tracking-wide">
        <span className={isUser ? "" : "text-blue-700 dark:text-blue-300"}>
          {isUser ? "you" : "assistant"}
        </span>
        {streaming && (
          <span className="bg-blue-500 inline-block h-1.5 w-1.5 animate-pulse rounded-full" />
        )}
        {!streaming && message.ts && (
          <span className="font-mono normal-case tracking-normal">
            {shortTime(message.ts)}
          </span>
        )}
      </div>
      <div className="prose prose-sm dark:prose-invert max-w-none break-words text-[12px] [&_code]:rounded [&_code]:bg-background/60 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[11px] [&_code]:break-all [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:text-sm [&_h2]:font-semibold [&_h3]:text-[13px] [&_h3]:font-semibold [&_li]:my-0 [&_ol]:my-1 [&_p]:my-1 [&_pre]:my-1.5 [&_pre]:overflow-x-auto [&_pre]:whitespace-pre-wrap [&_pre]:break-words [&_pre]:bg-background/60 [&_pre]:p-2 [&_pre]:text-[11px] [&_ul]:my-1">
        {message.text}
      </div>
    </li>
  );
}

// Subtle inline divider rendered when session-archived-then-resumed
// fires. Communicates that the session boundary was crossed and the
// agent is continuing with handover context rather than the live
// session, so the user isn't surprised by context differences.
function SessionArchivedDivider() {
  return (
    <li className="flex items-center gap-2 py-1">
      <div className="h-px flex-1 bg-border/40" />
      <span className="text-muted-foreground text-[9px] italic">
        session archived — continuing with handover context
      </span>
      <div className="h-px flex-1 bg-border/40" />
    </li>
  );
}
