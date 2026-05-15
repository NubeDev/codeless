import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  useEventStreamWithState,
  useJob,
  useRepos,
  useRpc,
  type Job,
  type JobId,
  type SseConnectionStatus,
} from "@/lib/rpc";

import { CostCell, WallClockCell } from "./JobRow";
import { JobChat } from "./RunPane";
import { SpecPane } from "./spec/SpecPane";
import { StageDetail } from "./StageDetail";
import { StagesOverview } from "./StagesOverview";
import { StatusBadge } from "./StatusBadge";
import {
  JobTabs,
  type ActiveTab,
  type StageTab,
} from "./JobTabs";

// ------------------------------------------------------------------ helpers

// Derive a short human title for the tab strip from the job's YAML name,
// branch slug, or id prefix — in that order of preference.
function friendlyTitle(job: Job): string {
  if (job.template_yaml) {
    const m = /^name:\s*(.+)$/m.exec(job.template_yaml);
    if (m) return m[1].trim().slice(0, 32);
  }
  if (job.branch) {
    const tail = job.branch.split("/").pop()?.trim();
    if (tail && tail.length > 0) {
      return tail.length > 32 ? `${tail.slice(0, 30)}…` : tail;
    }
  }
  return `Job ${job.id.slice(0, 6)}`;
}

// ------------------------------------------------------------------ component

interface Props {
  jobId: JobId;
  // Only the active tab writes the URL; inactive instances stay mounted
  // for instant tab-switching.
  active: boolean;
  onTitleResolved?: (title: string) => void;
  onOpenJobTab?: (jobId: JobId, title: string) => void;
}

export function JobPage({
  jobId,
  active,
  onTitleResolved,
  onOpenJobTab,
}: Props) {
  const { data: job, error, loading, refetch: refetchJob } = useJob(jobId);
  const { data: repos } = useRepos();

  // Active tab: starts on "Stages" (the default overview per JOB-UI.md).
  const [activeTab, setActiveTab] = useState<ActiveTab>({
    kind: "system",
    id: "Stages",
  });

  // Stage tabs opened by the user. Pinned tabs are persisted in
  // localStorage so they survive page reload. The key is scoped by
  // jobId so tabs from different jobs don't bleed together.
  const [stageTabs, setStageTabs] = useState<StageTab[]>(() => {
    try {
      const raw = localStorage.getItem(`codeless-pinned-tabs:${jobId}`);
      if (!raw) return [];
      const saved = JSON.parse(raw) as Array<{
        stageId: string;
        stageName: string;
      }>;
      return saved.map((t) => ({
        kind: "stage" as const,
        stageId: t.stageId,
        stageName: t.stageName,
        pinned: true,
      }));
    } catch {
      return [];
    }
  });

  // Surface the derived title once when it changes. Pin the callback to
  // a ref so the effect doesn't re-run on every arrow-function recreate.
  const titleCbRef = useRef(onTitleResolved);
  useEffect(() => {
    titleCbRef.current = onTitleResolved;
  }, [onTitleResolved]);
  const derivedTitle = job ? friendlyTitle(job) : null;
  useEffect(() => {
    if (derivedTitle) titleCbRef.current?.(derivedTitle);
  }, [derivedTitle]);

  // Persist pinned tabs to localStorage whenever the tab list changes.
  // Only pinned tabs are stored; unpinned tabs are transient.
  useEffect(() => {
    const pinned = stageTabs
      .filter((t) => t.pinned)
      .map((t) => ({ stageId: t.stageId, stageName: t.stageName }));
    localStorage.setItem(
      `codeless-pinned-tabs:${jobId}`,
      JSON.stringify(pinned),
    );
  }, [stageTabs, jobId]);

  // Live-refetch the job row on lifecycle events that mutate it.
  const sseStatus = useEventStreamWithState(
    { scope: "job", job_id: jobId },
    useCallback(
      (env) => {
        const t = env.event.type;
        if (
          t === "job-promoted" ||
          t === "job-started" ||
          t === "job-completed" ||
          t === "job-failed" ||
          t === "job-stopped" ||
          t === "task-completed"
        ) {
          refetchJob();
        }
      },
      [refetchJob],
    ),
  );

  // Tick `now` once per second while the job is live so the wall-clock
  // cell advances without a server round-trip.
  const isLive =
    job?.status === "running" ||
    job?.status === "queued" ||
    job?.status === "awaiting-review";
  const [now, setNow] = useState<number>(() => Date.now());
  useEffect(() => {
    if (!isLive) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [isLive]);

  // Open a Stage-N tab. Idempotent: clicking a row whose tab already
  // exists just activates it rather than duplicating.
  const handleOpenStageTab = useCallback(
    (stageId: string, stageName: string) => {
      setStageTabs((prev) => {
        if (prev.some((t) => t.stageId === stageId)) return prev;
        return [
          ...prev,
          { kind: "stage", stageId, stageName, pinned: false },
        ];
      });
      setActiveTab({ kind: "stage", stageId, stageName, pinned: false });
    },
    [],
  );

  const handleCloseStageTab = useCallback((stageId: string) => {
    setStageTabs((prev) => prev.filter((t) => t.stageId !== stageId));
    setActiveTab((current) => {
      if (current.kind === "stage" && current.stageId === stageId) {
        return { kind: "system", id: "Stages" };
      }
      return current;
    });
  }, []);

  const handleTogglePin = useCallback((stageId: string) => {
    setStageTabs((prev) =>
      prev.map((t) =>
        t.stageId === stageId ? { ...t, pinned: !t.pinned } : t,
      ),
    );
    // Also update the active tab object if it's the pinned one, so the
    // tab button's pinned visual reflects the change immediately.
    setActiveTab((current) => {
      if (current.kind === "stage" && current.stageId === stageId) {
        return { ...current, pinned: !current.pinned };
      }
      return current;
    });
  }, []);

  const repo = repos?.find((r) => r.id === job?.repo_id) ?? null;

  const specRefreshRef = useRef<(() => void) | null>(null);

  const handleRefetch = useCallback(() => {
    refetchJob();
    specRefreshRef.current?.();
  }, [refetchJob]);

  if (loading) {
    return (
      <div
        className={cn(
          "flex h-full items-center justify-center text-sm text-muted-foreground",
          !active && "hidden",
        )}
      >
        loading job…
      </div>
    );
  }
  if (error || !job) {
    return (
      <div
        className={cn(
          "flex h-full items-center justify-center text-sm text-destructive",
          !active && "hidden",
        )}
      >
        {error?.message ?? "Job not found"}
      </div>
    );
  }

  const title = friendlyTitle(job);

  return (
    <div className={cn("flex h-full min-h-0 flex-col", !active && "hidden")}>
      {/* Page header */}
      <PageHeader
        job={job}
        repoName={repo?.name ?? null}
        now={now}
        sseStatus={sseStatus}
        title={title}
        onOpenJobTab={onOpenJobTab}
        refetchJob={handleRefetch}
      />

      {/* Tab bar */}
      <JobTabs
        jobId={jobId}
        active={activeTab}
        stageTabs={stageTabs}
        onActivate={setActiveTab}
        onClose={handleCloseStageTab}
        onTogglePin={handleTogglePin}
      />

      {/* Tab content */}
      <div className="min-h-0 flex-1 overflow-hidden">
        {activeTab.kind === "system" && activeTab.id === "CHAT" && (
          <div className="flex h-full flex-col px-4 py-4 md:px-8">
            <JobChat
              job={job}
              uiLocation={`jobs/${job.id}`}
              refetchJob={refetchJob}
            />
          </div>
        )}
        {activeTab.kind === "system" && activeTab.id === "SPEC" && (
          <SpecPane jobId={jobId} refreshRef={specRefreshRef} />
        )}
        {activeTab.kind === "system" && activeTab.id === "Stages" && (
          <StagesOverview
            jobId={jobId}
            onOpenStageTab={handleOpenStageTab}
          />
        )}
        {activeTab.kind === "stage" && (
          <StageDetail
            jobId={jobId}
            stageId={activeTab.stageId}
            stageName={activeTab.stageName}
          />
        )}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ sub-components

const TERMINAL_STATUSES: Set<Job["status"]> = new Set([
  "completed",
  "failed",
  "stopped",
]);

function PageHeader({
  job,
  repoName,
  now,
  sseStatus,
  title,
  onOpenJobTab,
  refetchJob,
}: {
  job: Job;
  repoName: string | null;
  now: number;
  sseStatus: SseConnectionStatus;
  title: string;
  onOpenJobTab?: (jobId: JobId, title: string) => void;
  refetchJob: () => void;
}) {
  const rpc = useRpc();
  const [busy, setBusy] = useState<"stop" | "rerun" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const isTerminal = TERMINAL_STATUSES.has(job.status);
  const isRunning =
    job.status === "running" || job.status === "awaiting-review";

  const stop = async () => {
    setBusy("stop");
    setErr(null);
    try {
      await rpc.call("stop_job", { job_id: job.id });
      refetchJob();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const rerun = async () => {
    setBusy("rerun");
    setErr(null);
    try {
      const fresh = await rpc.call("rerun_job", { source_job_id: job.id });
      if (onOpenJobTab) {
        onOpenJobTab(fresh.id, title);
      } else {
        window.location.assign(`/jobs/${fresh.id}`);
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="border-border/50 shrink-0 border-b">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-2.5">
        <StatusBadge status={job.status} />
        <SseStatusDot status={sseStatus} />
        <h2 className="min-w-0 truncate text-sm font-semibold">{title}</h2>
        {repoName && (
          <span className="text-muted-foreground text-xs">{repoName}</span>
        )}
        <span
          className="text-muted-foreground hidden truncate font-mono text-[11px] md:inline"
          title={job.branch}
        >
          {job.branch}
        </span>
        <div className="ml-auto flex items-center gap-3">
          <WallClockCell
            startedAt={job.started_at}
            endedAt={job.ended_at}
            capMs={job.wall_clock_cap_ms}
            now={now}
          />
          <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
          {isRunning && (
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2.5 text-xs"
              onClick={() => void stop()}
              disabled={busy !== null}
            >
              {busy === "stop" ? "stopping…" : "stop"}
            </Button>
          )}
          {isTerminal && (
            <Button
              size="sm"
              variant="default"
              className="h-7 px-2.5 text-xs"
              onClick={() => void rerun()}
              disabled={busy !== null}
              title="Queue a fresh run with the same prompt, runner, and caps"
            >
              {busy === "rerun" ? "queuing…" : "re-run"}
            </Button>
          )}
        </div>
      </div>
      {err && (
        <div className="text-destructive border-border/50 border-t px-4 py-1 text-xs">
          {err}
        </div>
      )}
    </div>
  );
}

// Small liveness dot beside the status badge. Always rendered so its
// presence is not itself a signal; colour and animation convey state.
function SseStatusDot({ status }: { status: SseConnectionStatus }) {
  const { tone, label } = sseStatusVisual(status);
  return (
    <span
      className={cn(
        "inline-block h-2 w-2 shrink-0 rounded-full",
        tone,
        status.state === "reconnecting" && "animate-pulse",
      )}
      title={label}
      aria-label={label}
    />
  );
}

function sseStatusVisual(status: SseConnectionStatus): {
  tone: string;
  label: string;
} {
  switch (status.state) {
    case "connecting":
      return { tone: "bg-muted-foreground/60", label: "connecting…" };
    case "live":
      return { tone: "bg-emerald-500", label: "live" };
    case "reconnecting": {
      const s = Math.round(status.since_ms / 1000);
      return {
        tone: "bg-amber-500",
        label: s > 0 ? `reconnecting for ${s}s` : "reconnecting…",
      };
    }
    case "disconnected":
      return { tone: "bg-destructive", label: "disconnected" };
  }
}

