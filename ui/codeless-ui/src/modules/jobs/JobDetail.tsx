import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";
import { navigate } from "@/lib/route";
import { useJob, useRepos, useRpc, type JobId } from "@/lib/rpc";

import { CostCell } from "./JobRow";
import { FilesChanged } from "./FilesChanged";
import { JobTimeline } from "./JobTimeline";
import { ReviewPanel } from "./ReviewPanel";
import { StageTree } from "./StageTree";
import { StatusBadge } from "./StatusBadge";

// Side-panel content for a single selected job. The header surfaces
// every piece of "where did this actually happen" state the user
// needs in order to drop into a terminal and inspect: real branch
// (the value `WorktreeManager` actually created on disk and wrote
// back to the job row), preserved worktree path, repo's source
// checkout.
export function JobDetail({ jobId }: { jobId: JobId }) {
  const { data: job, error, loading } = useJob(jobId);
  const { data: repos } = useRepos();
  const rpc = useRpc();
  const [rerunning, setRerunning] = useState(false);
  const [rerunError, setRerunError] = useState<string | null>(null);
  const repo = job ? repos?.find((r) => r.id === job.repo_id) ?? null : null;

  // Cloning a job is cheap on the server (one row insert + one event)
  // and the user expectation is "land me on the new run immediately"
  // — navigate as soon as the new job id is back.
  const rerun = async () => {
    if (!job) return;
    setRerunning(true);
    setRerunError(null);
    try {
      const fresh = await rpc.call("rerun_job", { source_job_id: job.id });
      navigate(`/jobs/${fresh.id}`);
    } catch (e) {
      setRerunError(e instanceof Error ? e.message : String(e));
    } finally {
      setRerunning(false);
    }
  };

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
              {repo && (
                <span className="text-muted-foreground text-xs">
                  · {repo.name}
                </span>
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
              <span>
                <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
              </span>
            </div>
            {rerunError && (
              <div className="text-destructive mt-2 text-xs">
                re-run failed: {rerunError}
              </div>
            )}
            <div className="text-muted-foreground mt-1 font-mono text-[11px]">
              {job.id}
            </div>
            {job.stop_reason && (
              <div className="text-destructive mt-2 text-xs">
                stopped: <span className="font-mono">{job.stop_reason}</span>
              </div>
            )}
            <div className="mt-3 grid gap-1.5">
              <PathRow
                label="branch"
                value={job.branch}
                hint="git branch created in the source repo; survives worktree cleanup"
              />
              <PathRow
                label="worktree"
                value={job.worktree_path}
                hint={
                  job.worktree_path
                    ? "preserved on disk after the job ends — cd into it to inspect"
                    : "not provisioned yet (or running without --worktree-root)"
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
                <PathRow
                  label="repo"
                  value={repo.local_path}
                  hint="the source checkout this job's branch lives in"
                />
              )}
            </div>
            {job.prompt && (
              <p className="mt-3 line-clamp-3 text-sm">{job.prompt}</p>
            )}
          </>
        )}
      </div>
      <StageTree jobId={jobId} />
      <ReviewPanel jobId={jobId} />
      <Tabs defaultValue="timeline" className="flex min-h-0 flex-1 flex-col">
        <TabsList className="mx-3 mt-2 self-start">
          <TabsTrigger value="timeline" className="text-xs">
            Timeline
          </TabsTrigger>
          <TabsTrigger value="files" className="text-xs">
            Files changed
          </TabsTrigger>
        </TabsList>
        <TabsContent value="timeline" className="min-h-0 flex-1 mt-0">
          <JobTimeline jobId={jobId} />
        </TabsContent>
        <TabsContent value="files" className="min-h-0 flex-1 mt-0">
          <FilesChanged jobId={jobId} />
        </TabsContent>
      </Tabs>
    </div>
  );
}

// POSIX single-quote a string for safe paste into bash/zsh. Paths
// coming back from the runtime are well-formed but may contain
// spaces; quoting unconditionally costs nothing and stops a stray
// shell metacharacter from surprising someone who paste-and-runs.
function shellQuote(s: string): string {
  return `'${s.replace(/'/g, "'\\''")}'`;
}

function PathRow({
  label,
  value,
  hint,
  altCopy,
}: {
  label: string;
  value: string | null;
  hint: string;
  // Optional secondary copy action. The label appears on a small
  // button next to the primary "copy"; the value is what lands on
  // the clipboard. Used by the worktree row to offer
  // `cd <path> && git status` for terminal users without opening a
  // real terminal (desktop / browser parity — see SCOPE.md
  // "Rule 3: one UI framework forever").
  altCopy?: { label: string; value: string };
}) {
  return (
    <div className="flex items-start gap-2 text-xs">
      <span className="text-muted-foreground w-16 shrink-0 font-medium uppercase tracking-wide text-[10px] mt-1">
        {label}
      </span>
      <div className="min-w-0 flex-1">
        {value ? (
          <CopyField value={value} altCopy={altCopy} />
        ) : (
          <span className="text-muted-foreground italic">none</span>
        )}
        <div className="text-muted-foreground mt-0.5 text-[10px] leading-snug">
          {hint}
        </div>
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
      // Clipboard API is blocked in non-secure contexts; the value is
      // still visible in the inline <code> so the user can copy by
      // hand. Surfacing the failure as a toast adds more noise than
      // it removes — keep the UI silent.
    }
  };
  return (
    <div className="flex items-center gap-1.5">
      <code
        className={cn(
          "min-w-0 flex-1 truncate rounded border border-border/40 bg-muted/30 px-1.5 py-0.5 text-[11px] font-mono",
        )}
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

