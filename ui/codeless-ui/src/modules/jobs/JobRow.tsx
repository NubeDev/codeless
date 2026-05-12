import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useRpc, type Job } from "@/lib/rpc";
import { StatusBadge } from "./StatusBadge";
import { relativeTime } from "./eventFormat";

const TERMINAL: Set<Job["status"]> = new Set([
  "completed",
  "failed",
  "stopped",
]);

interface Props {
  job: Job;
  // Render-time clock used for the relative age label. Lifted to the
  // dashboard so a single 30s interval drives every row, instead of
  // each row holding its own timer.
  now: number;
  // Latest one-line activity chip ("Bash(git status)", "verify
  // failed (exit 1)", etc.) projected from the events stream. Null
  // when nothing eventful has arrived yet for this job.
  lastSummary: string | null;
  onSelect?: (job: Job) => void;
}

export function JobRow({ job, now, lastSummary, onSelect }: Props) {
  const rpc = useRpc();
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const stop = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setStopping(true);
    setError(null);
    try {
      await rpc.call("stop_job", { job_id: job.id });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setStopping(false);
    }
  };

  const canStop = !TERMINAL.has(job.status);
  const summary = job.prompt?.split("\n")[0] ?? "(template job)";
  // Pick the most informative age: when the job has terminated, the
  // user wants to know how long ago it ended; otherwise how long ago
  // it was created (or started, if known).
  const ageAnchor = job.ended_at ?? job.started_at ?? job.created_at;

  return (
    <div
      className="hover:bg-muted/40 flex cursor-pointer items-center gap-3 border-b border-border/50 px-3 py-2 text-sm last:border-b-0"
      onClick={() => onSelect?.(job)}
    >
      <StatusBadge status={job.status} />
      <div className="min-w-0 flex-1">
        <div className="truncate">{summary}</div>
        <div className="text-muted-foreground flex flex-wrap items-center gap-1.5 font-mono text-[11px]">
          <Badge
            variant="outline"
            className="h-4 px-1.5 py-0 text-[10px] font-normal"
          >
            {job.runner}
          </Badge>
          <span className="truncate">{job.branch || "(branch tbd)"}</span>
          <span>·</span>
          <span title={new Date(ageAnchor).toLocaleString()}>
            {relativeTime(ageAnchor, now)}
          </span>
        </div>
        {lastSummary && (
          <div
            className="text-muted-foreground/80 mt-0.5 truncate text-[11px] italic"
            title={lastSummary}
          >
            {lastSummary}
          </div>
        )}
        {error && <div className="text-destructive mt-1 text-xs">{error}</div>}
      </div>
      <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
      {canStop && (
        <Button size="sm" variant="ghost" onClick={stop} disabled={stopping}>
          {stopping ? "stopping…" : "stop"}
        </Button>
      )}
    </div>
  );
}

// Cost vs. cap. Renders "—" for jobs that haven't billed anything;
// flips to a warning tint above 80% of the cap so the dashboard
// surfaces approaching kills before they happen.
export function CostCell({ cost, cap }: { cost: number; cap: number }) {
  if (cost === 0 && cap === 0) {
    return <span className="text-muted-foreground font-mono text-[11px]">—</span>;
  }
  const ratio = cap > 0 ? cost / cap : 0;
  const warn = ratio >= 0.8;
  return (
    <span
      className={`font-mono text-[11px] ${warn ? "text-amber-500" : "text-muted-foreground"}`}
      title={`${formatCents(cost)} of ${formatCents(cap)} cap`}
    >
      {formatCents(cost)}
      {cap > 0 && <span className="opacity-60"> / {formatCents(cap)}</span>}
    </span>
  );
}

function formatCents(n: number): string {
  if (n < 100) return `${n}¢`;
  return `$${(n / 100).toFixed(2)}`;
}
