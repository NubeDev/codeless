import { useCallback, useEffect, useMemo, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { useEventStream, type EventEnvelope, type JobId } from "@/lib/rpc";

interface Props {
  jobId: JobId;
}

// Live timeline for a single job — subscribes to the per-job filter
// and groups events into stages and tasks so the panel reads like the
// runner's narrative ("verify failed in stage A's compile task") and
// not a wall of events. `ai-token` deltas accumulate into a single
// assistant-message bubble per owning task, rendered as light
// markdown. A "raw events" toggle drops back to the flat ordered
// stream for debugging the runtime's event order without rebuilding
// the grouper in your head.
export function JobTimeline({ jobId }: Props) {
  const [events, setEvents] = useState<EventEnvelope[]>([]);
  const [raw, setRaw] = useState(false);

  useEffect(() => {
    setEvents([]);
  }, [jobId]);

  useEventStream(
    { scope: "job", job_id: jobId },
    useCallback((env) => {
      setEvents((prev) => [...prev, env]);
    }, []),
  );

  const grouped = useMemo(() => groupEvents(events), [events]);

  return (
    <div className="flex h-full flex-col">
      <div className="border-border/50 flex items-center justify-end gap-2 border-b px-4 py-1.5">
        <Label htmlFor="timeline-raw" className="text-muted-foreground text-[11px]">
          raw events
        </Label>
        <Switch
          id="timeline-raw"
          checked={raw}
          onCheckedChange={setRaw}
          aria-label="show raw events"
        />
      </div>
      {events.length === 0 ? (
        <div className="text-muted-foreground p-4 text-center text-sm">
          waiting for events…
        </div>
      ) : raw ? (
        <ScrollArea className="flex-1">
          <ol className="space-y-2 p-4">
            {events.map((env) => (
              <RawItem key={env.cursor} env={env} />
            ))}
          </ol>
        </ScrollArea>
      ) : (
        <ScrollArea className="flex-1">
          <ol className="space-y-3 p-4">
            {grouped.map((g) => (
              <StageGroup key={g.key} group={g} />
            ))}
          </ol>
        </ScrollArea>
      )}
    </div>
  );
}

// Grouped shape. The flat envelope stream collapses into stages →
// tasks → (events | assistant-message bubble). Job-level events
// (queued / started / completed / stopped / failed / promoted) and
// any stage_id-less events get the synthetic JOB_GROUP_KEY so they
// render in a single "Job" header at the top of the list.
type GroupKey = string;
const JOB_GROUP_KEY: GroupKey = "__job__";

interface StageNode {
  key: GroupKey;
  stageId: string | null;
  events: EventEnvelope[];
  tasks: TaskNode[];
}

interface TaskNode {
  key: GroupKey;
  taskId: string;
  events: EventEnvelope[];
  // Accumulated assistant text across consecutive ai-token deltas.
  // Null when no token has arrived; an empty string remains null.
  assistantText: string | null;
  // The final cost line — only emitted once per task when an
  // ai-message-complete arrives. Bound here so the bubble can show
  // it under the prose.
  completion: AiCompletion | null;
}

interface AiCompletion {
  inputTokens: number;
  outputTokens: number;
  costCents: number;
}

function groupEvents(events: EventEnvelope[]): StageNode[] {
  const stages = new Map<GroupKey, StageNode>();
  const stageOrder: GroupKey[] = [];

  const stageFor = (envStageId: string | null): StageNode => {
    const key = envStageId ?? JOB_GROUP_KEY;
    let node = stages.get(key);
    if (!node) {
      node = { key, stageId: envStageId, events: [], tasks: [] };
      stages.set(key, node);
      stageOrder.push(key);
    }
    return node;
  };

  const taskFor = (stage: StageNode, taskId: string): TaskNode => {
    let task = stage.tasks.find((t) => t.taskId === taskId);
    if (!task) {
      task = {
        key: taskId,
        taskId,
        events: [],
        assistantText: null,
        completion: null,
      };
      stage.tasks.push(task);
    }
    return task;
  };

  for (const env of events) {
    const stage = stageFor(env.stage_id);
    const e = env.event;
    if (env.task_id) {
      const task = taskFor(stage, env.task_id);
      if (e.type === "ai-token") {
        task.assistantText = (task.assistantText ?? "") + e.delta;
        continue;
      }
      if (e.type === "ai-message-complete") {
        task.completion = {
          inputTokens: e.input_tokens,
          outputTokens: e.output_tokens,
          costCents: e.cost_cents,
        };
        continue;
      }
      task.events.push(env);
    } else {
      stage.events.push(env);
    }
  }

  return stageOrder.map((k) => stages.get(k)!);
}

function StageGroup({ group }: { group: StageNode }) {
  const isJob = group.stageId === null;
  return (
    <li className="border-border/60 rounded border bg-card/30 p-2.5">
      <div className="mb-2 flex items-baseline gap-2">
        <span className="text-xs font-semibold">
          {isJob ? "Job" : "Stage"}
        </span>
        {!isJob && (
          <span className="text-muted-foreground font-mono text-[10px]">
            {group.stageId}
          </span>
        )}
      </div>
      <ul className="space-y-1.5">
        {group.events.map((env) => (
          <EventRow key={env.cursor} env={env} />
        ))}
        {group.tasks.map((t) => (
          <TaskBlock key={t.key} task={t} />
        ))}
      </ul>
    </li>
  );
}

function TaskBlock({ task }: { task: TaskNode }) {
  return (
    <li className="border-border/50 ml-1 border-l pl-3">
      <div className="text-muted-foreground mb-1 font-mono text-[10px]">
        task {task.taskId}
      </div>
      <ul className="space-y-1">
        {task.events.map((env) => (
          <EventRow key={env.cursor} env={env} />
        ))}
      </ul>
      {(task.assistantText || task.completion) && (
        <AssistantBubble
          text={task.assistantText ?? ""}
          completion={task.completion}
        />
      )}
    </li>
  );
}

function AssistantBubble({
  text,
  completion,
}: {
  text: string;
  completion: AiCompletion | null;
}) {
  return (
    <div className="bg-muted/40 border-border/40 mt-2 rounded border px-2.5 py-1.5">
      <div className="text-muted-foreground mb-1 text-[10px] uppercase tracking-wide">
        Assistant output
      </div>
      {text && <Markdown source={text} />}
      {completion && (
        <div className="text-muted-foreground mt-1.5 border-t border-border/40 pt-1 text-[11px]">
          in {completion.inputTokens} · out {completion.outputTokens} · $
          {(completion.costCents / 100).toFixed(2)}
        </div>
      )}
    </div>
  );
}

function EventRow({ env }: { env: EventEnvelope }) {
  const time = new Date(env.created_at).toLocaleTimeString();
  return (
    <li className="border-border/30 border-l pl-2">
      <div className="flex items-baseline gap-2">
        <span className="text-muted-foreground font-mono text-[10px]">
          {time}
        </span>
        <span className="font-mono text-[11px] font-medium">
          {env.event.type}
        </span>
      </div>
      <PayloadLine env={env} />
    </li>
  );
}

function RawItem({ env }: { env: EventEnvelope }) {
  const time = new Date(env.created_at).toLocaleTimeString();
  return (
    <li className="border-border/50 border-l-2 pl-3">
      <div className="flex items-baseline gap-2">
        <span className="text-muted-foreground font-mono text-[11px]">
          {time}
        </span>
        <span className="font-mono text-xs font-medium">{env.event.type}</span>
      </div>
      <PayloadLine env={env} />
    </li>
  );
}

function PayloadLine({ env }: { env: EventEnvelope }) {
  const e = env.event;
  switch (e.type) {
    case "ai-token":
      return (
        <pre className="text-muted-foreground mt-0.5 max-h-32 overflow-hidden whitespace-pre-wrap text-xs">
          {e.delta}
        </pre>
      );
    case "ai-message-complete":
      return (
        <div className="text-muted-foreground text-xs">
          in {e.input_tokens} · out {e.output_tokens} · $
          {(e.cost_cents / 100).toFixed(2)}
        </div>
      );
    case "tool-call":
    case "tool-approval-requested":
      return <ToolCallLine tool={e.tool} argsJson={e.args_json} />;
    case "verify-failed":
      return (
        <div className="text-destructive text-xs">exit {e.exit_code}</div>
      );
    case "stage-completed":
    case "task-completed":
      return <div className="text-muted-foreground text-xs">{e.status}</div>;
    case "job-stopped":
      return <div className="text-muted-foreground text-xs">{e.reason}</div>;
    default:
      return null;
  }
}

// Light markdown: enough to make claude's prose readable without
// pulling react-markdown for what is currently a single render site.
// Recognises fenced code blocks (```lang\n…\n```), inline backtick
// spans, blank-line paragraph splits, and ``#``-prefixed headings.
// Anything richer (lists, links, tables) renders as plain text — the
// raw-events toggle is the escape hatch for content that needs a
// better view.
function Markdown({ source }: { source: string }) {
  const blocks = parseMarkdownBlocks(source);
  return (
    <div className="space-y-1.5 text-xs">
      {blocks.map((b, i) => {
        if (b.kind === "code") {
          return (
            <pre
              key={i}
              className="bg-background/60 border-border/40 overflow-x-auto rounded border px-2 py-1 font-mono text-[11px]"
            >
              {b.lang && (
                <div className="text-muted-foreground mb-0.5 text-[10px]">
                  {b.lang}
                </div>
              )}
              <code>{b.body}</code>
            </pre>
          );
        }
        if (b.kind === "heading") {
          return (
            <div key={i} className="text-sm font-semibold">
              {b.text}
            </div>
          );
        }
        return (
          <p key={i} className="whitespace-pre-wrap leading-snug">
            {renderInline(b.text)}
          </p>
        );
      })}
    </div>
  );
}

type MdBlock =
  | { kind: "code"; lang: string | null; body: string }
  | { kind: "heading"; text: string }
  | { kind: "paragraph"; text: string };

function parseMarkdownBlocks(source: string): MdBlock[] {
  const lines = source.split("\n");
  const out: MdBlock[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^```(.*)$/);
    if (fence) {
      const lang = fence[1].trim() || null;
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        body.push(lines[i]);
        i++;
      }
      if (i < lines.length) i++;
      out.push({ kind: "code", lang, body: body.join("\n") });
      continue;
    }
    if (/^#{1,6}\s+/.test(line)) {
      out.push({
        kind: "heading",
        text: line.replace(/^#{1,6}\s+/, ""),
      });
      i++;
      continue;
    }
    if (line.trim() === "") {
      i++;
      continue;
    }
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^```/.test(lines[i]) &&
      !/^#{1,6}\s+/.test(lines[i])
    ) {
      para.push(lines[i]);
      i++;
    }
    out.push({ kind: "paragraph", text: para.join("\n") });
  }
  return out;
}

// Inline parser: splits on ``backtick`` runs and wraps the inside in
// <code>. Everything else stays a literal string. Bold and italics
// are not recognised — they round-trip as their markdown source which
// is still readable.
function renderInline(text: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  const re = /`([^`]+)`/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    parts.push(
      <code
        key={`c${key++}`}
        className="bg-background/60 rounded px-1 font-mono text-[11px]"
      >
        {m[1]}
      </code>,
    );
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

function truncate(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

// Render a tool-call as `Tool(<summary>)` where the summary is the
// single most informative field from the tool's args. Falls back to
// truncated JSON for tools we don't recognise yet. The component is
// intentionally loose with types — args_json is whatever the upstream
// emitted; if a key isn't where we expect it, we land on the JSON
// fallback rather than crashing the timeline. The full args_json is
// always available in the row's title= tooltip for the curious.
function ToolCallLine({ tool, argsJson }: { tool: string; argsJson: string }) {
  const summary = summariseToolArgs(tool, argsJson);
  return (
    <div
      className="text-muted-foreground font-mono text-xs"
      title={argsJson || "(no args)"}
    >
      {tool}({summary})
    </div>
  );
}

function summariseToolArgs(tool: string, argsJson: string): string {
  if (!argsJson) return "";
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return truncate(argsJson, 80);
  }
  if (!parsed || typeof parsed !== "object") return truncate(argsJson, 80);
  const args = parsed as Record<string, unknown>;
  const pick = (key: string): string | null =>
    typeof args[key] === "string" ? (args[key] as string) : null;

  switch (tool) {
    case "Bash": {
      const cmd = pick("command");
      return cmd ? truncate(cmd, 80) : truncate(argsJson, 80);
    }
    case "Read":
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "NotebookEdit": {
      const path = pick("file_path") ?? pick("path") ?? pick("notebook_path");
      return path ? relativise(path) : truncate(argsJson, 80);
    }
    case "Glob": {
      const pattern = pick("pattern");
      return pattern ?? truncate(argsJson, 80);
    }
    case "Grep": {
      const pattern = pick("pattern");
      const path = pick("path");
      if (pattern && path) return `${pattern} in ${relativise(path)}`;
      return pattern ?? truncate(argsJson, 80);
    }
    case "AskUserQuestion": {
      const q = pick("question");
      return q ? `"${truncate(q, 60)}"` : truncate(argsJson, 80);
    }
    case "TodoWrite": {
      const todos = args.todos;
      if (Array.isArray(todos))
        return `${todos.length} item${todos.length === 1 ? "" : "s"}`;
      return truncate(argsJson, 80);
    }
    default:
      return truncate(argsJson, 80);
  }
}

// Strip the worktree prefix from a path so the timeline reads as
// `Read(src/main.rs)` rather than the full
// `/tmp/demo-target/.codeless/worktrees/job-<id>/src/main.rs`. The
// boundary marker is the `.codeless/worktrees/job-<id>/` segment —
// after that the rest is the repo-relative path the user thinks in.
function relativise(absolute: string): string {
  const m = absolute.match(/\.codeless\/worktrees\/job-[^/]+\/(.*)$/);
  return m ? m[1] : absolute;
}
