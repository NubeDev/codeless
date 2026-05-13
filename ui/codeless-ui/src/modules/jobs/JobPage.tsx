import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { navigate } from "@/lib/route";
import {
  useEventStream,
  useJob,
  useRepos,
  useRpc,
  type Job,
  type JobId,
  type Repo,
} from "@/lib/rpc";

import { FilesChanged } from "./FilesChanged";
import { HandoverPanel } from "./HandoverPanel";
import { SpecPane } from "./spec/SpecPane";
import { JobTimeline } from "./JobTimeline";
import { CostCell, WallClockCell } from "./JobRow";
import { ReviewPanel } from "./ReviewPanel";
import { RunPane } from "./RunPane";
import { StageDetail } from "./StageDetail";
import { StageTree } from "./StageTree";
import { StatusBadge } from "./StatusBadge";

// The job page is structured around three concerns the user has at
// distinct moments:
//
//   SPEC    — what *you* author. template.yaml, SCOPE.md, WORKFLOW.md,
//             extra docs. Editing here lands a commit in the source
//             repo via update_job_template / write_job_file. The
//             current run is unaffected; the next run reads the new
//             content.
//   STAGES  — the spine. Live stage list while running; click a stage
//             to drill into its rollup (duration, cost, and — once
//             the wishlist lands — session id, commits, tool ribbon,
//             final message).
//   RUN     — what the *runtime* produced. Timeline (raw event stream),
//             Files changed (diff against base), Handover (the
//             contract for the next session), Worktree (git surface).
//
// Selecting a stage replaces the right pane with `StageDetail` so the
// detail has the full real-estate; a "back" link in StageDetail
// returns to the regular section content.
type Section =
  | "stages"
  | "spec"
  | "status"
  | "timeline"
  | "files"
  | "handover"
  | "worktree";

interface RailSection {
  id: Section;
  label: string;
  hint?: string;
}
interface RailGroup {
  label: string;
  hint: string;
  items: RailSection[];
}

// Sub-rail groups. Hints were removed in the layout pass: the group
// labels are descriptive enough on their own, and the small grey
// subtitles cluttered the rail without adding information density.
const RAIL: RailGroup[] = [
  {
    label: "Spec",
    hint: "",
    items: [{ id: "spec", label: "Files" }],
  },
  {
    label: "Run",
    hint: "",
    items: [
      { id: "status", label: "Overview" },
      { id: "stages", label: "Stages" },
      { id: "timeline", label: "Timeline" },
      { id: "files", label: "Files changed" },
      { id: "handover", label: "Handover" },
      { id: "worktree", label: "Worktree" },
    ],
  },
];

interface Props {
  jobId: JobId;
  active: boolean;
  // Click handler invoked when the user clicks a file row in
  // "Files changed". Receives the absolute path inside the
  // worktree; the host (`App.tsx`) decides whether to open an
  // editor tab or show a side preview. Optional because the
  // dashboard-side singleton mount path doesn't need it.
  onOpenFile?: (path: string) => void;
  // Called when the page first resolves a job title we'd rather
  // see in the tab strip than "Job <ulid>". Driven by the YAML
  // template name (preferred) or the prompt's first line.
  onTitleResolved?: (title: string) => void;
  // Open a brand-new job-detail tab. Re-run uses this so the freshly
  // cloned job appears as its own tab rather than silently swapping
  // the current tab's URL (which the JobPage ignores — its jobId is
  // a prop from JobDetailStack, not a route parameter).
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
}

export function JobPage({
  jobId,
  active,
  onOpenFile,
  onTitleResolved,
  onOpenJobTab,
}: Props) {
  const { data: job, error, loading, refetch: refetchJob } = useJob(jobId);
  const { data: repos } = useRepos();
  const [section, setSection] = useState<Section>("status");
  const [selectedStageId, setSelectedStageId] = useState<string | null>(null);

  // Surface the resolved title once, when it changes. The parent
  // (`JobDetailStack`) typically passes an inline arrow, so we pin
  // the callback to a ref and only fire when the derived title
  // genuinely changes — depending on `onTitleResolved` directly
  // would re-fire every render, the parent's state update would
  // recreate the arrow, and the cycle would spin until React
  // aborted the tree (white screen).
  const titleCbRef = useRef(onTitleResolved);
  useEffect(() => {
    titleCbRef.current = onTitleResolved;
  }, [onTitleResolved]);
  const derivedTitle = job ? friendlyJobTitle(job) : null;
  useEffect(() => {
    if (derivedTitle) titleCbRef.current?.(derivedTitle);
  }, [derivedTitle]);

  // Switching jobs should drop any stage selection — the IDs come
  // from a different job entirely.
  useEffect(() => {
    setSelectedStageId(null);
  }, [jobId]);

  // Live-refetch the job row whenever the runtime emits a
  // status-changing event for it. Without this, clicking `[run]`
  // would flip status server-side but the header badge would stay
  // `draft` until reload — and a second click would 409. The cost is
  // one `get_job` per emitted lifecycle event for the focused job;
  // the dashboard already does the same calculus per row.
  useEventStream(
    { scope: "job", job_id: jobId },
    useCallback(
      (env) => {
        const t = env.event.type;
        if (
          t === "job-promoted" ||
          t === "job-started" ||
          t === "job-completed" ||
          t === "job-failed" ||
          t === "job-stopped"
        ) {
          refetchJob();
        }
      },
      [refetchJob],
    ),
  );

  // Keep the URL in sync with the active job tab so a reload or
  // shareable link lands the user back on the same view. Only the
  // active tab writes — inactive instances are still mounted (the
  // dashboard keeps them warm for cheap tab switches) and would
  // otherwise fight each other for the URL.
  useEffect(() => {
    if (!active) return;
    const want = `/jobs/${jobId}`;
    if (typeof window !== "undefined" && window.location.pathname !== want) {
      navigate(want);
    }
  }, [active, jobId]);

  const repo: Repo | null = useMemo(() => {
    if (!job || !repos) return null;
    return repos.find((r) => r.id === job.repo_id) ?? null;
  }, [job, repos]);

  // Wire the worktree → editor open path. File paths surfaced by
  // `job_diff` are repo-root relative; we resolve against the
  // worktree path (so the editor opens the version of the file
  // that this job actually edited, not the master checkout).
  const handleOpenFile = useCallback(
    (relPath: string) => {
      if (!onOpenFile || !job?.worktree_path) return;
      const abs = joinPath(job.worktree_path, relPath);
      onOpenFile(abs);
    },
    [onOpenFile, job?.worktree_path],
  );

  const onSelectSection = useCallback((s: Section) => {
    setSection(s);
    // Switching sections clears the stage drilldown so the user
    // doesn't see a stale "back to all stages" link in the wrong
    // section.
    setSelectedStageId(null);
  }, []);

  const onSelectStage = useCallback((stageId: string) => {
    setSection("stages");
    setSelectedStageId(stageId);
  }, []);

  if (loading) {
    return <CenteredMessage message="loading job…" tone="muted" />;
  }
  if (error) {
    return <CenteredMessage message={error.message} tone="error" />;
  }
  if (!job) return null;

  return (
    <div className={cn("flex h-full min-h-0 flex-col", !active && "hidden")}>
      <JobHeader
        job={job}
        repo={repo}
        onOpenJobTab={onOpenJobTab}
        refetchJob={refetchJob}
      />
      <div className="flex min-h-0 flex-1">
        <SubRail current={section} onSelect={onSelectSection} />
        <div className="min-w-0 flex-1 overflow-hidden">
          {section === "stages" && selectedStageId !== null ? (
            <StageDetail
              jobId={jobId}
              stageId={selectedStageId}
              onBack={() => setSelectedStageId(null)}
            />
          ) : (
            <>
              {section === "stages" && (
                <StagesSection
                  job={job}
                  selectedStageId={selectedStageId}
                  onSelectStage={onSelectStage}
                />
              )}
              {section === "status" && (
                <RunPane
                  job={job}
                  refetchJob={refetchJob}
                  onOpenJobTab={onOpenJobTab}
                  onEditSpec={() => onSelectSection("spec")}
                />
              )}
              {section === "timeline" && <JobTimeline jobId={jobId} />}
              {section === "files" && (
                <FilesChanged jobId={jobId} onOpenFile={handleOpenFile} />
              )}
              {section === "handover" && <HandoverPanel job={job} />}
              {section === "spec" && (
                <SpecPane jobId={job.id} onOpenFile={onOpenFile} />
              )}
              {section === "worktree" && <WorktreeSection job={job} repo={repo} />}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function CenteredMessage({
  message,
  tone,
}: {
  message: string;
  tone: "muted" | "error";
}) {
  return (
    <div
      className={cn(
        "flex h-full items-center justify-center text-sm",
        tone === "muted" ? "text-muted-foreground" : "text-destructive",
      )}
    >
      {message}
    </div>
  );
}

const TERMINAL_STATUSES: Set<Job["status"]> = new Set([
  "completed",
  "failed",
  "stopped",
]);

// Page header: identity, status, primary actions. The lifecycle
// timeline and per-phase actions stay in `RunPane`, but Stop and
// Re-run live up here because they're the actions a user reaches for
// on *any* section of the page (you don't want to navigate to
// "Overview" just to stop a runaway job).
function JobHeader({
  job,
  repo,
  onOpenJobTab,
  refetchJob,
}: {
  job: Job;
  repo: Repo | null;
  onOpenJobTab?: (jobId: JobId, initialTitle: string) => void;
  refetchJob: () => void;
}) {
  const rpc = useRpc();
  const [busy, setBusy] = useState<"stop" | "rerun" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const isTerminal = TERMINAL_STATUSES.has(job.status);
  const isRunning = job.status === "running" || job.status === "awaiting-review";
  const title = friendlyJobTitle(job);

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
        navigate(`/jobs/${fresh.id}`);
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
        <h2 className="min-w-0 truncate text-sm font-semibold">{title}</h2>
        {repo && (
          <span className="text-muted-foreground text-xs">
            {repo.name}
          </span>
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
            now={Date.now()}
          />
          <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
          {isRunning && (
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2.5 text-xs"
              onClick={stop}
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
              onClick={rerun}
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

function SubRail({
  current,
  onSelect,
}: {
  current: Section;
  onSelect: (s: Section) => void;
}) {
  return (
    <nav className="border-border/50 flex w-36 shrink-0 flex-col gap-4 overflow-y-auto border-r p-2 pt-3">
      {RAIL.map((group) => (
        <div key={group.label} className="space-y-0.5">
          <div className="text-muted-foreground px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider">
            {group.label}
          </div>
          <div className="flex flex-col gap-px">
            {group.items.map((item) => (
              <button
                key={item.id}
                type="button"
                onClick={() => onSelect(item.id)}
                className={cn(
                  "rounded px-2 py-1 text-left text-[13px] transition-colors",
                  current === item.id
                    ? "bg-accent text-foreground font-medium"
                    : "text-muted-foreground hover:bg-accent/40 hover:text-foreground",
                )}
              >
                {item.label}
              </button>
            ))}
          </div>
        </div>
      ))}
    </nav>
  );
}

function StagesSection({
  job,
  selectedStageId,
  onSelectStage,
}: {
  job: Job;
  selectedStageId: string | null;
  onSelectStage: (stageId: string) => void;
}) {
  return (
    <ScrollArea className="h-full">
      <div className="space-y-3 p-3">
        <StageTree
          jobId={job.id}
          templateYaml={job.template_yaml ?? null}
          selectedStageId={selectedStageId}
          onSelectStage={onSelectStage}
        />
        <ReviewPanel jobId={job.id} />
      </div>
    </ScrollArea>
  );
}

function WorktreeSection({ job, repo }: { job: Job; repo: Repo | null }) {
  return (
    <ScrollArea className="h-full">
      <div className="space-y-3 p-3 text-sm">
        <FactRow
          label="branch"
          value={job.branch}
          hint="git branch created in the source repo; survives worktree cleanup"
        />
        <FactRow
          label="worktree"
          value={job.worktree_path}
          hint={
            job.worktree_path
              ? "preserved on disk after the job ends — cd into it to inspect"
              : "not provisioned (the runner crashed before allocating a worktree)"
          }
          altCopy={
            job.worktree_path
              ? {
                  label: "cd",
                  value: `cd ${shellQuote(job.worktree_path)} && git status`,
                }
              : undefined
          }
        />
        {repo && (
          <FactRow
            label="repo"
            value={repo.local_path}
            hint="the source checkout this job's branch lives in"
          />
        )}
        <FactRow
          label="job id"
          value={job.id}
          hint="ULID — also the runs/<id>/ directory key for handover + log"
        />
      </div>
    </ScrollArea>
  );
}

function FactRow({
  label,
  value,
  hint,
  altCopy,
}: {
  label: string;
  value: string | null;
  hint: string;
  altCopy?: { label: string; value: string };
}) {
  return (
    <div className="space-y-1">
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </div>
      {value ? (
        <CopyField value={value} altCopy={altCopy} />
      ) : (
        <span className="text-muted-foreground italic">none</span>
      )}
      <div className="text-muted-foreground text-[10px] leading-snug">
        {hint}
      </div>
    </div>
  );
}

function CopyField({
  value,
  altCopy,
}: {
  value: string;
  altCopy?: { label: string; value: string };
}) {
  const [copied, setCopied] = useState<"primary" | "alt" | null>(null);
  const copy = async (text: string, which: "primary" | "alt") => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(which);
      window.setTimeout(() => setCopied(null), 1200);
    } catch {
      // Clipboard API blocked in non-secure contexts. The value is
      // visible in the inline <code> so manual selection still works;
      // a toast would add noise without removing the failure mode.
    }
  };
  return (
    <div className="flex items-center gap-1.5">
      <code
        className="border-border/40 bg-muted/30 min-w-0 flex-1 truncate rounded border px-1.5 py-0.5 font-mono text-[11px]"
        title={value}
      >
        {value}
      </code>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 shrink-0 px-2 text-[10px]"
        onClick={() => void copy(value, "primary")}
      >
        {copied === "primary" ? "copied" : "copy"}
      </Button>
      {altCopy && (
        <Button
          variant="ghost"
          size="sm"
          className="h-6 shrink-0 px-2 text-[10px]"
          title={`copy '${altCopy.value}'`}
          onClick={() => void copy(altCopy.value, "alt")}
        >
          {copied === "alt" ? "copied" : altCopy.label}
        </Button>
      )}
    </div>
  );
}

// Pick a short, descriptive title for the tab strip. Order of
// preference: template name (most informative — it's the user's
// own label), then the first line of the prompt (truncated), then a
// fallback "Job …". Reading template_yaml inline rather than parsing
// it server-side keeps this surface in the browser; the body is at
// most ~10 lines so a regex is fine.
function friendlyJobTitle(job: Job): string {
  if (job.template_yaml) {
    const match = /^name:\s*(.+)$/m.exec(job.template_yaml);
    if (match) {
      return match[1].trim().slice(0, 32);
    }
  }
  // Prefer the branch's slug tail over the prompt's first line: a
  // mock job's prompt is often multi-paragraph runner instructions
  // that look terrible truncated to a tab title. The branch is
  // already a stable, short, user-meaningful identifier
  // (`codeless/job-<slug>` or `codeless/<runner>-<rand>`).
  if (job.branch) {
    const tail = job.branch.split("/").pop()?.trim();
    if (tail && tail.length > 0) {
      return tail.length > 32 ? `${tail.slice(0, 30)}…` : tail;
    }
  }
  return `Job ${job.id.slice(0, 6)}`;
}

function joinPath(base: string, rel: string): string {
  if (rel.startsWith("/")) return rel;
  if (base.endsWith("/")) return `${base}${rel}`;
  return `${base}/${rel}`;
}

function shellQuote(s: string): string {
  return `'${s.replace(/'/g, "'\\''")}'`;
}
