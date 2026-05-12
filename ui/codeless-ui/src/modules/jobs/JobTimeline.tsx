import { useCallback, useEffect, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { useEventStream, type EventEnvelope, type JobId } from "@/lib/rpc";

interface Props {
  jobId: JobId;
}

// Live timeline for a single job — subscribes to the per-job filter
// and renders events in arrival order. Token deltas (`ai-token`) are
// coalesced into the previous task-bound block so the panel doesn't
// scroll past on every chunk.
export function JobTimeline({ jobId }: Props) {
  const [events, setEvents] = useState<EventEnvelope[]>([]);

  // Reset the buffer when the watched job changes — otherwise switching
  // selection shows the previous job's stream until the new one fills in.
  useEffect(() => {
    setEvents([]);
  }, [jobId]);

  useEventStream(
    { scope: "job", job_id: jobId },
    useCallback((env) => {
      setEvents((prev) => {
        // Coalesce consecutive ai-token events on the same task into one
        // accumulated entry. Keeps the visible list short during a hot
        // streaming response.
        const last = prev[prev.length - 1];
        if (
          last &&
          last.event.type === "ai-token" &&
          env.event.type === "ai-token" &&
          last.task_id === env.task_id
        ) {
          const merged: EventEnvelope = {
            ...env,
            event: {
              ...env.event,
              delta: last.event.delta + env.event.delta,
            },
          };
          return [...prev.slice(0, -1), merged];
        }
        return [...prev, env];
      });
    }, []),
  );

  if (events.length === 0) {
    return (
      <div className="text-muted-foreground p-4 text-center text-sm">
        waiting for events…
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <ol className="space-y-2 p-4">
        {events.map((env) => (
          <TimelineItem key={env.cursor} env={env} />
        ))}
      </ol>
    </ScrollArea>
  );
}

function TimelineItem({ env }: { env: EventEnvelope }) {
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
          in {e.input_tokens} · out {e.output_tokens} · ${(e.cost_cents / 100).toFixed(2)}
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
      return (
        <div className="text-muted-foreground text-xs">{e.status}</div>
      );
    case "job-stopped":
      return (
        <div className="text-muted-foreground text-xs">{e.reason}</div>
      );
    default:
      return null;
  }
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
      if (Array.isArray(todos)) return `${todos.length} item${todos.length === 1 ? "" : "s"}`;
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
