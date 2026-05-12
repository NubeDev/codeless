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
      return (
        <div className="text-muted-foreground font-mono text-xs">
          {e.tool}({truncate(e.args_json, 80)})
        </div>
      );
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
