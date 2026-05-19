// Settings -> Workspaces tab. Smaller landing surface ahead of the
// full `/workspaces` route (§"Milestone 4" of
// DOCS/WORKSPACE-ATTACH.md). Renders the attached-workspaces table
// with active-dot + open/detach affordances, plus the `+ Attach`
// button that opens `AttachWorkspaceDialog`. The empty state and the
// table share the same hydration path through `useWorkspacesSync` so
// list and empty render from a single source of truth.

import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import type { AttachedWorkspace, RepoId } from "@/lib/rpc/wire";
import { cn } from "@/lib/utils";
import { AttachWorkspaceDialog } from "@/modules/workspaces/AttachWorkspaceDialog";
import { DetachWorkspaceDialog } from "@/modules/workspaces/DetachWorkspaceDialog";
import { EmptyWorkspacesState } from "@/modules/workspaces/EmptyWorkspacesState";
import { ImportJobDialog } from "@/modules/workspaces/ImportJobDialog";
import { useWorkspacesStore } from "@/modules/workspaces/store";
import { useWorkspacesSync } from "@/modules/workspaces/useWorkspacesSync";

import { SectionHeader } from "../components/SectionHeader";

export function WorkspacesSection() {
  useWorkspacesSync();
  const workspaces = useWorkspacesStore((s) => s.workspaces);
  const status = useWorkspacesStore((s) => s.status);
  const error = useWorkspacesStore((s) => s.error);
  const activeRepoId = useWorkspacesStore((s) => s.activeRepoId);
  const setActive = useWorkspacesStore((s) => s.setActive);

  const [attachOpen, setAttachOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [detachTarget, setDetachTarget] = useState<AttachedWorkspace | null>(
    null,
  );

  const activeWorkspace =
    workspaces.find((w) => w.repo_id === activeRepoId) ?? null;

  const showEmpty =
    status === "ready" && workspaces.length === 0;
  const showSpinner = status === "loading" && workspaces.length === 0;

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-start justify-between gap-3">
        <SectionHeader
          title="Workspaces"
          description="Attach a directory on disk to let the editor and the runner reach it. Detach to take it out of the editor's view."
        />
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={() => setImportOpen(true)}
            disabled={activeRepoId === null}
            data-testid="workspaces-import-job-button"
            title={
              activeRepoId === null
                ? "Activate a workspace to import a Job"
                : "Import a .codeless-job bundle into the active workspace"
            }
          >
            Import Job…
          </Button>
          <Button
            type="button"
            onClick={() => setAttachOpen(true)}
            data-testid="workspaces-attach-button"
          >
            + Attach
          </Button>
        </div>
      </div>

      {error ? (
        <p
          className="text-xs text-destructive"
          data-testid="workspaces-hydration-error"
        >
          Failed to load workspaces: {error}
        </p>
      ) : null}

      {showSpinner ? (
        <div className="flex justify-center py-6">
          <Spinner />
        </div>
      ) : showEmpty ? (
        <EmptyWorkspacesState onAttachClick={() => setAttachOpen(true)} />
      ) : (
        <WorkspaceTable
          workspaces={workspaces}
          activeRepoId={activeRepoId}
          onOpen={setActive}
          onDetach={setDetachTarget}
        />
      )}

      <AttachWorkspaceDialog
        open={attachOpen}
        onOpenChange={setAttachOpen}
      />
      <ImportJobDialog
        workspaceId={activeWorkspace?.repo_id ?? null}
        workspaceName={activeWorkspace?.repo_name ?? null}
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={(result) => {
          // Navigate to the new Job. Warnings ride along in the URL
          // hash so JobPage can read them on mount and render the
          // dismissible banner per §"UI / Imported-Job badge".
          const hash =
            result.warnings.length > 0
              ? `#imported-warnings=${encodeURIComponent(
                  JSON.stringify(result.warnings.map((w) => w.message)),
                )}`
              : "";
          window.location.assign(`/jobs/${result.job_id}${hash}`);
        }}
      />
      <DetachWorkspaceDialog
        workspace={detachTarget}
        onClose={() => setDetachTarget(null)}
      />
    </div>
  );
}

interface WorkspaceTableProps {
  workspaces: AttachedWorkspace[];
  activeRepoId: RepoId | null;
  onOpen(repoId: RepoId): void;
  onDetach(workspace: AttachedWorkspace): void;
}

function WorkspaceTable({
  workspaces,
  activeRepoId,
  onOpen,
  onDetach,
}: WorkspaceTableProps) {
  // Render newest-attached first so a fresh attach lands at the top.
  // Tie-break on repo_id keeps the order deterministic for tests.
  const rows = [...workspaces].sort((a, b) => {
    if (a.attached_at !== b.attached_at) return b.attached_at - a.attached_at;
    return a.repo_id < b.repo_id ? -1 : 1;
  });
  return (
    <div
      className="rounded-2xl border border-border/60 bg-card/40"
      data-testid="workspaces-table"
    >
      {rows.map((w) => {
        const isActive = w.repo_id === activeRepoId;
        return (
          <div
            key={w.repo_id}
            data-testid={`workspaces-row-${w.repo_id}`}
            data-active={isActive ? "true" : undefined}
            className="flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-b-0"
          >
            <span
              aria-label={isActive ? "active workspace" : "attached workspace"}
              data-testid={`workspaces-dot-${w.repo_id}`}
              className={cn(
                "size-2 shrink-0 rounded-full",
                isActive ? "bg-primary" : "bg-muted-foreground/30",
              )}
            />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium">{w.repo_name}</div>
              <div className="truncate text-xs text-muted-foreground">
                {w.fs_root}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Button
                type="button"
                size="sm"
                variant={isActive ? "secondary" : "outline"}
                onClick={() => onOpen(w.repo_id)}
                disabled={isActive}
                data-testid={`workspaces-open-${w.repo_id}`}
              >
                {isActive ? "Open" : "Open"}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => onDetach(w)}
                data-testid={`workspaces-detach-${w.repo_id}`}
              >
                Detach
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
