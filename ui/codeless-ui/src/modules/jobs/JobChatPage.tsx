import { useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  useJob,
  useRepos,
  useRpc,
  type Job,
  type JobId,
  type Repo,
} from "@/lib/rpc";

import { FilesChanged } from "./FilesChanged";
import { HandoverPanel } from "./HandoverPanel";
import { CostCell, WallClockCell } from "./JobRow";
import { JobTimeline } from "./JobTimeline";
import { ReviewPanel } from "./ReviewPanel";
import { JobChat, RunStrip } from "./RunPane";
import { SpecPane } from "./spec/SpecPane";
import { StageTree } from "./StageTree";
import { StatusBadge } from "./StatusBadge";

type SidebarTab = "summary" | "files" | "timeline" | "handover" | "stages";
type MainView = "chat" | "spec";

export function JobChatPage({ jobId }: { jobId: JobId }) {
  const { data: job, error, loading, refetch: refetchJob } = useJob(jobId);
  const { data: repos } = useRepos();
  const rpc = useRpc();
  const [tab, setTab] = useState<SidebarTab>("summary");
  const [view, setView] = useState<MainView>("chat");
  const [rerunning, setRerunning] = useState(false);
  const [rerunError, setRerunError] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const repo = job ? repos?.find((r) => r.id === job.repo_id) ?? null : null;

  const rerun = async () => {
    if (!job) return;
    setRerunning(true);
    setRerunError(null);
    try {
      const fresh = await rpc.call("rerun_job", { source_job_id: job.id });
      window.location.assign(`/jobs/${fresh.id}`);
    } catch (e) {
      setRerunError(e instanceof Error ? e.message : String(e));
    } finally {
      setRerunning(false);
    }
  };

  if (loading) {
    return <div className="text-muted-foreground p-8">Loading…</div>;
  }
  if (error || !job) {
    return (
      <div className="text-destructive p-8">
        {error?.message || "Job not found"}
      </div>
    );
  }

  const title =
    job.template_yaml?.match(/^name:\s*(.+)$/m)?.[1] || job.branch || job.id;

  return (
    <div className="flex h-full min-h-0 w-full overflow-hidden">
      <main className="bg-background flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b px-4 md:px-6">
          <StatusBadge status={job.status} />
          <span className="truncate text-sm font-semibold">{title}</span>
          <div className="min-w-0 flex-1 overflow-hidden">
            <RunStrip job={job} onEditSpec={() => setView("spec")} />
          </div>
          <div className="bg-muted/40 ml-2 inline-flex shrink-0 rounded-md p-0.5 text-xs">
            <button
              className={cn(
                "rounded px-3 py-1",
                view === "chat"
                  ? "bg-background shadow-sm font-medium"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => setView("chat")}
            >
              Chat
            </button>
            <button
              className={cn(
                "rounded px-3 py-1",
                view === "spec"
                  ? "bg-background shadow-sm font-medium"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => setView("spec")}
              title="Edit template.yaml, SCOPE.md, WORKFLOW.md and supporting docs"
            >
              Spec
            </button>
          </div>
          <button
            className="bg-card/80 border-border ml-2 shrink-0 rounded border px-2 py-1 text-xs md:hidden"
            onClick={() => setSidebarOpen(true)}
            aria-label="Open job details"
          >
            Details
          </button>
        </div>
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {view === "chat" ? (
            <div className="flex min-h-0 min-w-0 flex-1 flex-col px-4 py-4 md:px-8">
              <JobChat
                job={job}
                uiLocation={`jobs/${job.id}`}
                refetchJob={refetchJob}
              />
            </div>
          ) : (
            <div className="min-h-0 min-w-0 flex-1 overflow-hidden">
              <SpecPane jobId={jobId} />
            </div>
          )}
        </div>
      </main>

      <aside className="bg-card/80 hidden w-[340px] shrink-0 flex-col overflow-hidden border-l md:flex">
        <SidebarContent
          job={job}
          repo={repo}
          rerun={rerun}
          rerunning={rerunning}
          rerunError={rerunError}
          tab={tab}
          setTab={setTab}
          jobId={jobId}
        />
      </aside>

      {sidebarOpen && (
        <div className="fixed inset-0 z-50 flex md:hidden">
          <div
            className="absolute inset-0 bg-black/40"
            onClick={() => setSidebarOpen(false)}
          />
          <aside className="bg-card animate-in slide-in-from-right relative ml-auto flex h-full w-[90vw] max-w-xs flex-col border-l duration-200">
            <button
              className="bg-muted border-border absolute right-2 top-2 rounded border px-2 py-1 text-xs"
              onClick={() => setSidebarOpen(false)}
              aria-label="Close job details"
            >
              Close
            </button>
            <SidebarContent
              job={job}
              repo={repo}
              rerun={rerun}
              rerunning={rerunning}
              rerunError={rerunError}
              tab={tab}
              setTab={setTab}
              jobId={jobId}
            />
          </aside>
        </div>
      )}
    </div>
  );
}

interface SidebarContentProps {
  job: Job;
  repo: Repo | null;
  rerun: () => void;
  rerunning: boolean;
  rerunError: string | null;
  tab: SidebarTab;
  setTab: (tab: SidebarTab) => void;
  jobId: JobId;
}

function SidebarContent({
  job,
  repo,
  rerun,
  rerunning,
  rerunError,
  tab,
  setTab,
  jobId,
}: SidebarContentProps) {
  return (
    <>
      <div className="border-b p-4">
        <div className="mb-2 flex items-center gap-2">
          <span className="font-mono text-xs">{job.runner}</span>
          {repo && (
            <span className="text-muted-foreground text-xs">· {repo.name}</span>
          )}
          <Button
            size="sm"
            variant="outline"
            className="ml-auto h-6 px-2 text-[11px]"
            onClick={rerun}
            disabled={rerunning}
            title="Queue a fresh run with the same prompt, runner, and caps"
          >
            {rerunning ? "queuing…" : "re-run"}
          </Button>
        </div>
        <div className="mb-2 flex items-center gap-2">
          <WallClockCell
            startedAt={job.started_at}
            endedAt={job.ended_at}
            capMs={job.wall_clock_cap_ms}
            now={Date.now()}
          />
          <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
        </div>
        <div className="text-muted-foreground truncate font-mono text-[11px]">
          {job.id}
        </div>
        {rerunError && (
          <div className="text-destructive mt-2 text-xs">
            re-run failed: {rerunError}
          </div>
        )}
        {job.stop_reason && (
          <div className="text-destructive mt-2 text-xs">
            stopped: <span className="font-mono">{job.stop_reason}</span>
          </div>
        )}
        <div className="mt-3 grid gap-1.5">
          <SidebarPath label="branch" value={job.branch} />
          <SidebarPath label="worktree" value={job.worktree_path} />
          {repo && <SidebarPath label="repo" value={repo.local_path} />}
        </div>
        {job.prompt && (
          <p className="mt-3 line-clamp-3 text-sm">{job.prompt}</p>
        )}
      </div>
      <nav className="flex gap-1 border-b px-4 py-2 text-xs">
        <SidebarTabButton current={tab} value="summary" onSelect={setTab}>
          Summary
        </SidebarTabButton>
        <SidebarTabButton current={tab} value="files" onSelect={setTab}>
          Files
        </SidebarTabButton>
        <SidebarTabButton current={tab} value="timeline" onSelect={setTab}>
          Timeline
        </SidebarTabButton>
        <SidebarTabButton current={tab} value="handover" onSelect={setTab}>
          Handover
        </SidebarTabButton>
        <SidebarTabButton current={tab} value="stages" onSelect={setTab}>
          Stages
        </SidebarTabButton>
      </nav>
      <div className="flex-1 overflow-y-auto p-4">
        {tab === "summary" && <ReviewPanel jobId={jobId} />}
        {tab === "files" && <FilesChanged jobId={jobId} />}
        {tab === "timeline" && <JobTimeline jobId={jobId} />}
        {tab === "handover" && <HandoverPanel job={job} />}
        {tab === "stages" && <StageTree jobId={jobId} />}
      </div>
    </>
  );
}

function SidebarTabButton({
  current,
  value,
  onSelect,
  children,
}: {
  current: SidebarTab;
  value: SidebarTab;
  onSelect: (tab: SidebarTab) => void;
  children: React.ReactNode;
}) {
  return (
    <button
      className={cn(
        "rounded px-2 py-1",
        current === value && "bg-accent font-semibold",
      )}
      onClick={() => onSelect(value)}
    >
      {children}
    </button>
  );
}

function SidebarPath({
  label,
  value,
}: {
  label: string;
  value?: string | null;
}) {
  if (!value) return null;
  return (
    <div className="flex min-w-0 items-start gap-2 text-xs">
      <span className="text-muted-foreground w-16 shrink-0">{label}</span>
      <span className="min-w-0 flex-1 truncate font-mono" title={value}>
        {value}
      </span>
    </div>
  );
}
