import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { navigate } from "@/lib/route";
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
import { JobTimeline } from "./JobTimeline";
import { CostCell, WallClockCell } from "./JobRow";
import { ReviewPanel } from "./ReviewPanel";
import { StageTree } from "./StageTree";
import { StatusBadge } from "./StatusBadge";

// Sub-rail sections. Order matches the natural read of a job: what
// stages, then the live event stream, then the diff, then the
// handover, then the template/worktree metadata. The active section
// is parked in the URL search-string so reload restores the same
// view and a deep-link to "open this job at the handover" is one
// route away.
type Section =
  | "stages"
  | "timeline"
  | "files"
  | "handover"
  | "yaml"
  | "worktree";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "stages", label: "Stages" },
  { id: "timeline", label: "Timeline" },
  { id: "files", label: "Files changed" },
  { id: "handover", label: "Handover" },
  { id: "yaml", label: "Template" },
  { id: "worktree", label: "Worktree" },
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
}

export function JobPage({ jobId, active, onOpenFile, onTitleResolved }: Props) {
  const { data: job, error, loading } = useJob(jobId);
  const { data: repos } = useRepos();
  const rpc = useRpc();
  const [section, setSection] = useState<Section>("stages");

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

  if (loading) {
    return <CenteredMessage message="loading job…" tone="muted" />;
  }
  if (error) {
    return <CenteredMessage message={error.message} tone="error" />;
  }
  if (!job) return null;

  return (
    <div className={cn("flex h-full min-h-0 flex-col", !active && "hidden")}>
      <JobHeader job={job} repo={repo} rpc={rpc} />
      <div className="flex min-h-0 flex-1">
        <SubRail current={section} onSelect={setSection} />
        <div className="min-w-0 flex-1 overflow-hidden">
          {section === "stages" && <StagesSection job={job} />}
          {section === "timeline" && <JobTimeline jobId={jobId} />}
          {section === "files" && (
            <FilesChanged jobId={jobId} onOpenFile={handleOpenFile} />
          )}
          {section === "handover" && <HandoverPanel job={job} />}
          {section === "yaml" && <YamlSection job={job} />}
          {section === "worktree" && <WorktreeSection job={job} repo={repo} />}
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

function JobHeader({
  job,
  repo,
  rpc,
}: {
  job: Job;
  repo: Repo | null;
  rpc: ReturnType<typeof useRpc>;
}) {
  const [busy, setBusy] = useState<"stop" | "rerun" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const isTerminal =
    job.status === "completed" ||
    job.status === "failed" ||
    job.status === "stopped";

  const stop = async () => {
    setBusy("stop");
    setError(null);
    try {
      await rpc.call("stop_job", { job_id: job.id });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };
  const rerun = async () => {
    setBusy("rerun");
    setError(null);
    try {
      const fresh = await rpc.call("rerun_job", { source_job_id: job.id });
      navigate(`/jobs/${fresh.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="border-border/50 flex shrink-0 flex-wrap items-center gap-2 border-b px-4 py-2">
      <StatusBadge status={job.status} />
      <Badge variant="outline" className="font-mono text-[10px]">
        {job.runner}
      </Badge>
      {repo && (
        <span className="text-muted-foreground text-xs">{repo.name}</span>
      )}
      <span className="text-muted-foreground truncate font-mono text-[11px]">
        {job.branch}
      </span>
      <div className="ml-auto flex items-center gap-2">
        <WallClockCell
          startedAt={job.started_at}
          endedAt={job.ended_at}
          capMs={job.wall_clock_cap_ms}
          now={Date.now()}
        />
        <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
        {!isTerminal && (
          <Button
            size="sm"
            variant="outline"
            onClick={stop}
            disabled={busy !== null}
          >
            {busy === "stop" ? "stopping…" : "stop"}
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          onClick={rerun}
          disabled={busy !== null}
          title="Queue a fresh run with the same prompt, runner, caps"
        >
          {busy === "rerun" ? "queuing…" : "re-run"}
        </Button>
      </div>
      {error && (
        <div className="text-destructive w-full text-xs">{error}</div>
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
    <nav className="border-border/50 flex w-36 shrink-0 flex-col gap-0.5 border-r p-2">
      {SECTIONS.map((s) => (
        <button
          key={s.id}
          type="button"
          onClick={() => onSelect(s.id)}
          className={cn(
            "rounded px-2 py-1 text-left text-xs",
            current === s.id
              ? "bg-accent text-foreground"
              : "text-muted-foreground hover:bg-accent/40 hover:text-foreground",
          )}
        >
          {s.label}
        </button>
      ))}
    </nav>
  );
}

function StagesSection({ job }: { job: Job }) {
  return (
    <ScrollArea className="h-full">
      <div className="space-y-3 p-3">
        <StageTree jobId={job.id} templateYaml={job.template_yaml ?? null} />
        <ReviewPanel jobId={job.id} />
      </div>
    </ScrollArea>
  );
}

function YamlSection({ job }: { job: Job }) {
  if (!job.template_yaml) {
    return (
      <div className="text-muted-foreground p-4 text-sm italic">
        This job was a single-prompt run — no template YAML.
        {job.prompt && (
          <pre className="bg-muted/30 border-border/40 mt-3 whitespace-pre-wrap rounded border p-3 not-italic text-foreground text-xs leading-snug">
            {job.prompt}
          </pre>
        )}
      </div>
    );
  }
  return (
    <ScrollArea className="h-full">
      <div className="space-y-2 p-3">
        <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
          template_yaml
        </div>
        <pre className="bg-muted/30 border-border/40 whitespace-pre-wrap rounded border p-3 font-mono text-xs leading-snug">
          {job.template_yaml}
        </pre>
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
              : "not provisioned (mock runner or pre-run crash)"
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
  if (job.prompt) {
    const firstLine = job.prompt.split("\n")[0].trim();
    if (firstLine.length > 0) {
      return firstLine.length > 40
        ? `${firstLine.slice(0, 37)}…`
        : firstLine;
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
