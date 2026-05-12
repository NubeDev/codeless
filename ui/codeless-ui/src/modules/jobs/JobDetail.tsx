import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";
import { useJob, useRepos, type Job, type JobId } from "@/lib/rpc";

import { CostCell } from "./JobRow";
import { FilesChanged } from "./FilesChanged";
import { JobTimeline } from "./JobTimeline";
import { ReviewPanel } from "./ReviewPanel";
import { StatusBadge } from "./StatusBadge";

// Side-panel content for a single selected job. The header surfaces
// every piece of "where did this actually happen" state the user
// needs in order to drop into a terminal and inspect: real branch,
// preserved worktree path, repo's source checkout. The runtime-side
// `WorktreeManager` always names the branch `codeless/job-<job_id>`
// (the `Job.branch` column is currently a vestigial user-submitted
// field — until landing 6 honours it or drops it, this component
// shows the canonical name).
export function JobDetail({ jobId }: { jobId: JobId }) {
  const { data: job, error, loading } = useJob(jobId);
  const { data: repos } = useRepos();
  const repo = job ? repos?.find((r) => r.id === job.repo_id) ?? null : null;

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
              <span className="ml-auto">
                <CostCell cost={job.cost_cents} cap={job.cost_cap_cents} />
              </span>
            </div>
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
                value={canonicalBranch(job)}
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

// SCOPE.md "Workspace = one git worktree per job": the runtime names
// the branch `codeless/job-<job_id>` regardless of what was passed in
// `SubmitJobArgs.branch`. Derive it client-side so the UI never shows
// a name that no `git` command would resolve.
function canonicalBranch(job: Job): string {
  return `codeless/job-${job.id}`;
}

function PathRow({
  label,
  value,
  hint,
}: {
  label: string;
  value: string | null;
  hint: string;
}) {
  return (
    <div className="flex items-start gap-2 text-xs">
      <span className="text-muted-foreground w-16 shrink-0 font-medium uppercase tracking-wide text-[10px] mt-1">
        {label}
      </span>
      <div className="min-w-0 flex-1">
        {value ? (
          <CopyField value={value} />
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

function CopyField({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
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
        onClick={() => void onCopy()}
      >
        {copied ? "copied" : "copy"}
      </Button>
    </div>
  );
}

