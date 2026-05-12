import { useCallback, useState } from "react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useEventStream,
  useJobs,
  useRepos,
  type Job,
  type JobId,
  type Repo,
} from "@/lib/rpc";

import { JobDetail } from "./JobDetail";
import { JobRow } from "./JobRow";
import { SubmitJobDialog } from "./SubmitJobDialog";

// Repo-grouped jobs list — first user-visible Phase 2 surface. Loads
// repos + jobs once, then keeps the jobs map fresh by subscribing to
// the all-events stream and reacting to job-* envelopes. SCOPE.md Rule
// 4 (events drive state) is the design here: nothing polls.

export function JobsDashboard() {
  const repos = useRepos();
  const jobs = useJobs();
  const [overlay, setOverlay] = useState<Map<string, Job>>(new Map());
  const [selectedJobId, setSelectedJobId] = useState<JobId | null>(null);

  // Apply event deltas on top of the initial list_jobs snapshot. We
  // don't refetch — the event payload is enough for the columns we
  // render. If we add fields the events don't carry, swap to refetch.
  useEventStream(
    { scope: "all" },
    useCallback((env) => {
      if (!env.job_id) return;
      const e = env.event;
      if ("job_id" in e === false) return;
      setOverlay((prev) => {
        const next = new Map(prev);
        const existing = next.get(env.job_id!) ?? findJob(jobs.data, env.job_id!);
        if (!existing) return prev;
        const updated = applyEvent(existing, e);
        if (updated === existing) return prev;
        next.set(env.job_id!, updated);
        return next;
      });
    }, [jobs.data]),
  );

  if (repos.loading || jobs.loading) {
    return (
      <div className="space-y-3 p-4">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (repos.error || jobs.error) {
    return (
      <div className="text-destructive p-4 text-sm">
        {(repos.error ?? jobs.error)?.message}
      </div>
    );
  }
  if (!repos.data || !jobs.data) return null;

  const merged = jobs.data.map((j) => overlay.get(j.id) ?? j);
  const grouped = groupByRepo(merged, repos.data);
  const today = dayBucket(Date.now());
  const dailyTotal = merged
    .filter((j) => j.created_at >= today)
    .reduce((sum, j) => sum + j.cost_cents, 0);
  const activeCount = merged.filter(
    (j) => j.status === "running" || j.status === "queued",
  ).length;

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
      <header className="flex items-baseline justify-between">
        <h1 className="text-xl font-semibold">Jobs</h1>
        <div className="text-muted-foreground flex items-baseline gap-3 font-mono text-xs">
          <span>{activeCount} active</span>
          <span>·</span>
          <span>{merged.length} total</span>
          <span>·</span>
          <span title="Cost across jobs created today">today {formatCents(dailyTotal)}</span>
        </div>
      </header>
      <Sheet
        open={selectedJobId !== null}
        onOpenChange={(open) => !open && setSelectedJobId(null)}
      >
        <SheetContent
          side="right"
          className="flex w-full flex-col p-0 sm:max-w-xl"
        >
          <SheetHeader className="sr-only">
            <SheetTitle>Job detail</SheetTitle>
            <SheetDescription>
              Live timeline of events for the selected job.
            </SheetDescription>
          </SheetHeader>
          {selectedJobId && <JobDetail jobId={selectedJobId} />}
        </SheetContent>
      </Sheet>
      {grouped.map(({ repo, jobs }) => (
        <Card key={repo.id}>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 py-3">
            <CardTitle className="font-mono text-sm">{repo.name}</CardTitle>
            <SubmitJobDialog repo={repo} />
          </CardHeader>
          <CardContent className="p-0">
            {jobs.length === 0 ? (
              <div className="text-muted-foreground px-3 py-6 text-center text-sm">
                no jobs yet
              </div>
            ) : (
              jobs.map((j) => (
                <JobRow
                  key={j.id}
                  job={j}
                  onSelect={(job) => setSelectedJobId(job.id)}
                />
              ))
            )}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

function findJob(jobs: Job[] | null, id: string): Job | undefined {
  return jobs?.find((j) => j.id === id);
}

// Midnight in the operator's local timezone. Phase 3 uses the
// dashboard's render-time clock; cost caps in `codeless-runtime`
// already track per-day totals authoritatively — this is for at-a-
// glance UI only.
function dayBucket(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function formatCents(n: number): string {
  if (n < 100) return `${n}¢`;
  return `$${(n / 100).toFixed(2)}`;
}

function groupByRepo(
  jobs: Job[],
  repos: Repo[],
): Array<{ repo: Repo; jobs: Job[] }> {
  return repos.map((repo) => ({
    repo,
    jobs: jobs.filter((j) => j.repo_id === repo.id),
  }));
}

// Project an event onto the job snapshot. Returns the same instance
// when nothing changed so React skips the render.
function applyEvent(job: Job, e: { type: string }): Job {
  switch (e.type) {
    case "job-started":
      return { ...job, status: "running", started_at: Date.now() };
    case "job-completed":
      return { ...job, status: "completed", ended_at: Date.now() };
    case "job-failed":
      return { ...job, status: "failed", ended_at: Date.now() };
    case "job-stopped":
      return {
        ...job,
        status: "stopped",
        stop_reason:
          "reason" in e ? (e as { reason: Job["stop_reason"] }).reason : "user",
        ended_at: Date.now(),
      };
    case "job-promoted":
      return { ...job, status: "running" };
    default:
      return job;
  }
}
