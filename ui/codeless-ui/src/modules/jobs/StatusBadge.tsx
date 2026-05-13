import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { JobStatus } from "@/lib/rpc";

const TONE: Record<JobStatus, string> = {
  // Draft is the editable pre-run state — visually softer than queued
  // so the user can tell at a glance that nothing is running yet.
  draft: "bg-muted/50 text-muted-foreground italic",
  queued: "bg-muted text-muted-foreground",
  running: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
  "awaiting-review": "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  completed: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  failed: "bg-red-500/15 text-red-700 dark:text-red-300",
  stopped: "bg-zinc-500/15 text-zinc-700 dark:text-zinc-300",
};

export function StatusBadge({ status }: { status: JobStatus }) {
  return (
    <Badge variant="outline" className={cn("font-mono text-xs", TONE[status])}>
      {status}
    </Badge>
  );
}
