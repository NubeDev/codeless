import { useJob, type JobId } from "@/lib/rpc";

import { CostCell } from "./JobRow";
import { JobTimeline } from "./JobTimeline";
import { ReviewPanel } from "./ReviewPanel";
import { StatusBadge } from "./StatusBadge";

// Side-panel content for a single selected job. The header pulls fresh
// metadata via `get_job` (so badges reflect the latest snapshot if the
// dashboard list is stale); the timeline lives off the per-job event
// subscription and is always live.
export function JobDetail({ jobId }: { jobId: JobId }) {
  const { data: job, error, loading } = useJob(jobId);

  return (
    <div className="flex h-full flex-col">
      <div className="border-border/50 border-b p-4">
        {loading && (
          <div className="text-muted-foreground text-sm">loading…</div>
        )}
        {error && (
          <div className="text-destructive text-sm">{error.message}</div>
        )}
        {job && (
          <>
            <div className="flex items-center gap-2">
              <StatusBadge status={job.status} />
              <span className="font-mono text-xs">{job.runner}</span>
              <span className="text-muted-foreground font-mono text-xs">
                · {job.branch}
              </span>
              <span className="ml-auto">
                <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
              </span>
            </div>
            <div className="text-muted-foreground mt-2 font-mono text-[11px]">
              {job.id}
            </div>
            {job.prompt && (
              <p className="mt-3 line-clamp-3 text-sm">{job.prompt}</p>
            )}
          </>
        )}
      </div>
      <ReviewPanel jobId={jobId} />
      <div className="min-h-0 flex-1">
        <JobTimeline jobId={jobId} />
      </div>
    </div>
  );
}
