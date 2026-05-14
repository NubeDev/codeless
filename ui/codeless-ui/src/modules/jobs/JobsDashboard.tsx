import { useCallback, useEffect, useRef, useState } from "react";

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
  useRpc,
  type Job,
  type JobId,
  type Repo,
} from "@/lib/rpc";
import { navigate, useRoute } from "@/lib/route";

import { JobDetail } from "./JobDetail";
import { JobRow } from "./JobRow";
import { SubmitJobDialog } from "./SubmitJobDialog";
import { WorktreeGcButton } from "./WorktreeGcButton";
import { summariseEnvelope } from "./eventFormat";

// Repo-grouped jobs list — first user-visible Phase 2 surface. Loads
// repos + jobs once, then keeps the jobs map fresh by subscribing to
// the all-events stream and reacting to job-* envelopes. SCOPE.md Rule
// 4 (events drive state) is the design here: nothing polls.

// `/jobs/:id` opens the detail sheet on that job; `/jobs` (or any
// other route) closes it. Keeping the selection in the URL means a
// reload restores the same view, and bookmarking a job's detail page
// works without an in-app share affordance.
const JOB_ID_FROM_ROUTE = /^\/jobs\/([A-Z0-9]+)$/;

interface JobsDashboardProps {
  /**
   * Open a job in a dedicated workspace tab. When provided, clicking
   * a row in the dashboard takes this path instead of the legacy
   * right-side sheet. App.tsx supplies it; tests / Storybook can
   * omit it to keep the sheet behaviour visible.
   */
  onOpenJob?: (job: Job) => void;
}

export function JobsDashboard({ onOpenJob }: JobsDashboardProps = {}) {
  const repos = useRepos();
  const jobs = useJobs();
  const rpc = useRpc();
  const [overlay, setOverlay] = useState<Map<string, Job>>(new Map());
  // Mirror overlay into a ref so the event-stream callback can read
  // the latest map without depending on the state value (which would
  // re-create the callback on every overlay change and resubscribe
  // SSE on every event).
  const overlayRef = useRef(overlay);
  overlayRef.current = overlay;
  const [lastSummaries, setLastSummaries] = useState<
    Map<string, { text: string; at: number }>
  >(new Map());
  const route = useRoute();
  const matched = JOB_ID_FROM_ROUTE.exec(route.pathname);
  // The legacy sheet path keys off the route. When `onOpenJob` is
  // wired, we never set this URL anymore so the sheet stays closed —
  // the job-detail tab is the canonical workspace surface.
  const selectedJobId = (
    onOpenJob ? null : matched ? matched[1] : null
  ) as JobId | null;
  const openJob = useCallback(
    (job: Job) => {
      if (onOpenJob) {
        onOpenJob(job);
      } else {
        navigate(`/jobs/${job.id}`);
      }
    },
    [onOpenJob],
  );
  const closeJob = useCallback(() => navigate("/jobs"), []);

  // Tick a "now" clock every 30s so relative ages re-render without
  // each JobRow holding its own interval. 30s is the coarsest cadence
  // that still keeps "just now" -> "1m ago" transitions visible.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(t);
  }, []);

  // Apply event deltas on top of the initial list_jobs snapshot. We
  // don't refetch — the event payload is enough for the columns we
  // render. If we add fields the events don't carry, swap to refetch.
  // Separately, track each job's most recent eventful envelope so the
  // dashboard row can show a one-line activity chip without opening
  // the detail sheet. ai-token noise is filtered by summariseEnvelope
  // returning null on those.
  useEventStream(
    { scope: "all" },
    useCallback((env) => {
      if (!env.job_id) return;
      const e = env.event;
      const summary = summariseEnvelope(env);
      if (summary !== null) {
        setLastSummaries((prev) => {
          const next = new Map(prev);
          next.set(env.job_id!, { text: summary, at: env.created_at });
          return next;
        });
      }
      if ("job_id" in e === false) return;
      const existingInList =
        overlayRef.current.get(env.job_id!) ?? findJob(jobs.data, env.job_id!);
      // Unknown job — the dashboard's `useJobs` snapshot was loaded
      // before this row existed. `useJobs` is fetch-once, so without
      // this branch a freshly submitted job stays invisible until the
      // user navigates away and back. Fetch the row and seed the
      // overlay so the dashboard renders it immediately.
      if (!existingInList) {
        if (e.type === "job-queued") {
          rpc
            .call("get_job", { job_id: env.job_id! })
            .then((job) => {
              setOverlay((prev) => {
                const next = new Map(prev);
                next.set(job.id, job);
                return next;
              });
            })
            .catch(() => {
              // Best-effort: if the fetch fails the user can still
              // see the job after a manual reload.
            });
        }
        return;
      }
      setOverlay((prev) => {
        const next = new Map(prev);
        const updated = applyEvent(existingInList, e);
        if (updated === existingInList) return prev;
        next.set(env.job_id!, updated);
        return next;
      });
    }, [jobs.data, rpc]),
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

  const existingIds = new Set(jobs.data.map((j) => j.id));
  const merged = [
    ...jobs.data.map((j) => overlay.get(j.id) ?? j),
    // Append jobs discovered via SSE that weren't in the initial fetch.
    ...[...overlay.values()].filter((j) => !existingIds.has(j.id)),
  ];
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
      <header className="flex items-baseline justify-between gap-3">
        <h1 className="text-xl font-semibold">Jobs</h1>
        <div className="text-muted-foreground flex items-baseline gap-3 font-mono text-xs">
          <span>{activeCount} active</span>
          <span>·</span>
          <span>{merged.length} total</span>
          <span>·</span>
          <span title="Cost across jobs created today">today {formatCents(dailyTotal)}</span>
          <WorktreeGcButton />
        </div>
      </header>
      <Sheet
        open={selectedJobId !== null}
        onOpenChange={(open) => !open && closeJob()}
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
      {repos.data.length === 0 && <NoReposCta />}
      {repos.data.length > 0 && merged.length === 0 && <NoJobsCta />}
      {grouped.map(({ repo, jobs }) => (
        <Card key={repo.id}>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 py-3">
            <CardTitle className="font-mono text-sm">{repo.name}</CardTitle>
            <div className="flex items-center gap-2">
              <SubmitJobDialog repo={repo} />
            </div>
          </CardHeader>
          <CardContent className="p-0">
            {jobs.length === 0 ? (
              <div className="text-muted-foreground px-3 py-6 text-center text-sm">
                no jobs in this repo yet — click "new job"
              </div>
            ) : (
              jobs.map((j) => (
                <JobRow
                  key={j.id}
                  job={j}
                  now={now}
                  lastSummary={lastSummaries.get(j.id)?.text ?? null}
                  onSelect={(job) => openJob(job)}
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

// Empty state when repos exist but no jobs have been queued. Each
// repo card has its own "new job" button — name it explicitly so a
// fresh operator does not have to guess where the action lives.
function NoJobsCta() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">No jobs yet</CardTitle>
      </CardHeader>
      <CardContent className="text-muted-foreground space-y-2 text-sm">
        <p>
          Repos are registered but no jobs have been queued. Click{" "}
          <span className="font-medium">new job</span> on any repo card
          below to submit one.
        </p>
      </CardContent>
    </Card>
  );
}

// Empty-state shown when `list_repos` comes back empty. The UI has no
// repo-add affordance yet (Phase 2 follow-up); the CLI is the only
// path. This CTA names it so a fresh operator is not stranded at a
// blank dashboard.
function NoReposCta() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">No repos yet</CardTitle>
      </CardHeader>
      <CardContent className="text-muted-foreground space-y-2 text-sm">
        <p>
          The core has no repositories registered. Add one from the
          CLI:
        </p>
        <pre className="bg-muted overflow-x-auto rounded p-2 font-mono text-xs">
{"codeless --db <path> repos add <name> --clone-url <git-url> --local-path <abs-path>"}
        </pre>
        <p>
          Then refresh this page. Submit jobs from each repo's card.
        </p>
      </CardContent>
    </Card>
  );
}
