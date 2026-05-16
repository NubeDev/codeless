import { useCallback, useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useRepos,
  useRpc,
  type ProposedPatchListEntry,
  type ProposedScopePatch,
  type RepoId,
  type ScopePatchId,
} from "@/lib/rpc";
import { getCrossWindowEvents } from "@/lib/shell";
import { PatchCard } from "@/modules/jobs/patches/PatchCard";
import { ApproveDiffDialog } from "@/modules/jobs/patches/ApproveDiffDialog";
import { UndoToast } from "@/modules/jobs/patches/UndoToast";
import {
  renderProposalMarkdown,
  type PatchProposal,
  type PatchResolution,
} from "@/modules/jobs/patches/proposal";
import {
  SCOPE_PATCH_RESOLVED_EVENT,
  type ScopePatchResolvedPayload,
} from "@/modules/jobs/patches";

import {
  applyAllFilters,
  groupRows,
  PATCH_KIND_FILTERS,
  sortByNewest,
  type GroupBy,
  type PatchKindFilter,
  type PatchListRow,
} from "./filters";
import { usePatchQueue } from "./usePatchQueue";

// Surface C — the cross-workspace patch worklist. Reuses the per-job
// `PatchCard` so visual + interaction parity is automatic; this page
// adds the toolbar (filters / group-by / age toggle), the per-repo
// section headers, and the resolution-RPC wiring that needs the
// worklist's own `repo_id` instead of `useJob`'s.

// Local in-memory edits to a proposal's body. Edits made through the
// card's Edit affordance only become durable when the operator
// follows up with Approve (the runtime's `edit_scope_patch` only
// writes when paired with the approval commit, per decision OQ#3).
type EditedProposals = Map<ScopePatchId, PatchProposal>;

interface ApproveDiffState {
  repoId: RepoId;
  patchId: ScopePatchId;
  original: string;
  edited: string;
}

interface ToastState {
  repoId: RepoId;
  patchId: ScopePatchId;
  commitSha: string;
  expiresAt: number;
}

export function PatchesPage() {
  const rpc = useRpc();
  const { entries, loading, error, refetch } = usePatchQueue(null);
  const { data: repos } = useRepos();

  const [kindFilters, setKindFilters] = useState<Set<PatchKindFilter>>(
    () => new Set(),
  );
  const [target, setTarget] = useState("");
  const [showOlder, setShowOlder] = useState(false);
  const [groupBy, setGroupBy] = useState<GroupBy>("repo");

  const [editedProposals, setEditedProposals] = useState<EditedProposals>(
    () => new Map(),
  );
  const [approveDiff, setApproveDiff] = useState<ApproveDiffState | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);

  // Track resolutions that happened inside this window. Rows are
  // dropped optimistically (the queue file no longer carries the
  // entry), but the resolved-row collapse stays visible until the
  // user navigates away — matches the per-job inbox's behaviour.
  const [localResolutions, setLocalResolutions] = useState<
    Map<ScopePatchId, PatchResolution>
  >(() => new Map());

  const now = useMemo(() => Date.now(), [entries]);

  const rows: PatchListRow[] = useMemo(() => {
    if (entries === null) return [];
    const base: PatchListRow[] = entries.map((e) => ({
      repo_id: e.repo_id,
      patch: editedProposals.has(e.patch.id)
        ? mergeEdited(e.patch, editedProposals.get(e.patch.id)!)
        : e.patch,
    }));
    const filtered = applyAllFilters(base, {
      kinds: kindFilters,
      target,
      showOlderThan14Days: showOlder,
      now,
    });
    return sortByNewest(filtered);
  }, [entries, editedProposals, kindFilters, target, showOlder, now]);

  const groups = useMemo(() => groupRows(rows, groupBy), [rows, groupBy]);
  const totalVisible = rows.length;
  const totalAll = entries?.length ?? 0;

  const repoLabel = useCallback(
    (repoId: string) => {
      const r = repos?.find((x) => x.id === repoId);
      return r?.name ?? repoId;
    },
    [repos],
  );

  const broadcastResolved = useCallback(
    (payload: ScopePatchResolvedPayload) => {
      void getCrossWindowEvents().emit(SCOPE_PATCH_RESOLVED_EVENT, payload);
    },
    [],
  );

  const handleApprove = useCallback(
    async (
      repoId: RepoId,
      patchId: ScopePatchId,
      opts?: { editedBody?: string },
    ) => {
      if (opts?.editedBody !== undefined) {
        const editRes = await rpc.call("edit_scope_patch", {
          repo_id: repoId,
          patch_id: patchId,
          rendered: opts.editedBody,
        });
        if (editRes.outcome === "already_resolved") {
          recordResolution(patchId, editRes.resolution, editRes.commit_sha);
          refetch();
          return;
        }
      }
      const result = await rpc.call("approve_scope_patch", {
        repo_id: repoId,
        patch_id: patchId,
      });
      if (result.outcome === "approved") {
        recordResolution(patchId, "approved", result.commit_sha);
        if (opts?.editedBody === undefined) {
          setToast({
            repoId,
            patchId,
            commitSha: result.commit_sha,
            expiresAt: Date.now() + 10_000,
          });
        }
        broadcastResolved({
          patch_id: patchId,
          resolution: "approved",
          commit_sha: result.commit_sha,
        });
        refetch();
      } else if (result.outcome === "already_resolved") {
        recordResolution(patchId, result.resolution, result.commit_sha);
        refetch();
      }
    },
    [rpc, refetch, broadcastResolved],
  );

  const handleReject = useCallback(
    async (repoId: RepoId, patchId: ScopePatchId) => {
      const result = await rpc.call("reject_scope_patch", {
        repo_id: repoId,
        patch_id: patchId,
      });
      if (result.outcome === "rejected") {
        recordResolution(patchId, "rejected", result.commit_sha);
        broadcastResolved({
          patch_id: patchId,
          resolution: "rejected",
          commit_sha: result.commit_sha,
        });
        refetch();
      } else if (result.outcome === "already_resolved") {
        recordResolution(patchId, result.resolution, result.commit_sha);
        refetch();
      }
    },
    [rpc, refetch, broadcastResolved],
  );

  function recordResolution(
    patchId: ScopePatchId,
    kind: "approved" | "rejected" | "reverted",
    commitSha: string | undefined | null,
  ) {
    setLocalResolutions((curr) => {
      const next = new Map(curr);
      next.set(patchId, { kind, commit_sha: commitSha ?? "" } as PatchResolution);
      return next;
    });
  }

  const handleApproveAfterEdit = useCallback(
    (repoId: RepoId, patchId: ScopePatchId, original: string, edited: string) => {
      setApproveDiff({ repoId, patchId, original, edited });
    },
    [],
  );

  const handleApproveDiffConfirm = useCallback(async () => {
    if (!approveDiff) return;
    const { repoId, patchId, edited } = approveDiff;
    setApproveDiff(null);
    await handleApprove(repoId, patchId, { editedBody: edited });
  }, [approveDiff, handleApprove]);

  const handleEditSaved = useCallback(
    (patchId: ScopePatchId, updated: PatchProposal) => {
      setEditedProposals((curr) => {
        const next = new Map(curr);
        next.set(patchId, updated);
        return next;
      });
    },
    [],
  );

  const handleUndo = useCallback(async () => {
    if (!toast) return;
    const { repoId, patchId, commitSha } = toast;
    setToast(null);
    const result = await rpc.call("revert_scope_patch", {
      repo_id: repoId,
      commit_sha: commitSha,
    });
    recordResolution(patchId, "reverted", result.commit_sha);
    broadcastResolved({
      patch_id: patchId,
      resolution: "reverted",
      commit_sha: result.commit_sha,
    });
    refetch();
  }, [rpc, refetch, toast, broadcastResolved]);

  // Expire the undo toast 10s after it appears, mirroring the per-job
  // inbox (decision OQ#3).
  useToastExpiry(toast, setToast);

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <Toolbar
        kindFilters={kindFilters}
        onToggleKind={(b) =>
          setKindFilters((curr) => {
            const next = new Set(curr);
            if (next.has(b)) next.delete(b);
            else next.add(b);
            return next;
          })
        }
        target={target}
        onTarget={setTarget}
        showOlder={showOlder}
        onToggleShowOlder={() => setShowOlder((v) => !v)}
        groupBy={groupBy}
        onGroupBy={setGroupBy}
        countVisible={totalVisible}
        countTotal={totalAll}
        loading={loading}
        onRefresh={refetch}
      />

      <ScrollArea className="flex-1">
        <div className="mx-auto flex max-w-4xl flex-col gap-4 px-4 py-4">
          {error && (
            <p className="text-destructive text-sm">
              Failed to load patch queue: {error.message}
            </p>
          )}
          {!loading && entries !== null && totalAll === 0 && (
            <EmptyState
              message="No proposed patches across any repo."
              hint="REVIEW gates that emit patches will show up here."
            />
          )}
          {!loading && entries !== null && totalAll > 0 && totalVisible === 0 && (
            <EmptyState
              message="No patches match the active filters."
              hint={
                showOlder
                  ? "Try clearing the kind or target filters."
                  : "Toggle 'show older than 14 days' to see stale proposals."
              }
            />
          )}

          {groups.map((g) => (
            <section key={g.key} className="space-y-2">
              <GroupHeader
                label={
                  groupBy === "repo" ? repoLabel(g.key) : g.key
                }
                count={g.rows.length}
                groupBy={groupBy}
              />
              {g.rows.map((row) => {
                const proposal = liftToProposal(row.patch);
                const proposedAt =
                  row.patch.proposed_at ?? Date.now();
                const resolution = localResolutions.get(row.patch.id) ?? null;
                return (
                  <PatchCard
                    key={row.patch.id}
                    proposal={proposal}
                    proposedAt={proposedAt}
                    resolution={resolution}
                    onApprove={() => handleApprove(row.repo_id, row.patch.id)}
                    onReject={() => handleReject(row.repo_id, row.patch.id)}
                    onApproveAfterEdit={(edited) =>
                      handleApproveAfterEdit(
                        row.repo_id,
                        row.patch.id,
                        renderProposalMarkdown(proposal),
                        edited,
                      )
                    }
                    onEditSaved={(updated) =>
                      handleEditSaved(row.patch.id, updated)
                    }
                  />
                );
              })}
            </section>
          ))}
        </div>
      </ScrollArea>

      {toast && (
        <UndoToast
          commitSha={toast.commitSha}
          onUndo={() => void handleUndo()}
          onDismiss={() => setToast(null)}
        />
      )}
      {approveDiff && (
        <ApproveDiffDialog
          original={approveDiff.original}
          edited={approveDiff.edited}
          onCancel={() => setApproveDiff(null)}
          onConfirm={() => void handleApproveDiffConfirm()}
        />
      )}
    </div>
  );
}

// Lift a queue snapshot row into the card's proposal shape. The queue
// format does not carry `review_id` / `stage_id` (those are durably
// linked to the originating REVIEW gate via the SSE event, not via
// the markdown round-trip), so the card-required fields are filled
// in with empty placeholders — the worklist's edit path passes them
// straight back to the runtime, which re-parses and supplies its own
// validation.
function liftToProposal(p: ProposedScopePatch): PatchProposal {
  return {
    id: p.id,
    review_id: "" as PatchProposal["review_id"],
    stage_id: "" as PatchProposal["stage_id"],
    kind: p.kind,
    target: p.target,
    target_path: p.target_path,
    evidence_stage_id: p.evidence_stage_id ?? null,
    has_predicate: p.has_predicate,
    rationale: p.rationale,
    body: p.body,
  };
}

// Apply a local edit to a queue row. Only fields the editor can
// change are merged; identity / target metadata is preserved from
// the runtime snapshot so the row's RPC arguments stay correct.
function mergeEdited(
  snapshot: ProposedScopePatch,
  edited: PatchProposal,
): ProposedScopePatch {
  return {
    ...snapshot,
    rationale: edited.rationale,
    body: edited.body,
    has_predicate: edited.has_predicate,
    evidence_stage_id: edited.evidence_stage_id ?? undefined,
  };
}

function useToastExpiry(
  toast: ToastState | null,
  setToast: (t: ToastState | null) => void,
) {
  useEffect(() => {
    if (!toast) return;
    const remaining = toast.expiresAt - Date.now();
    if (remaining <= 0) {
      setToast(null);
      return;
    }
    const id = setTimeout(() => setToast(null), remaining);
    return () => clearTimeout(id);
  }, [toast, setToast]);
}

interface ToolbarProps {
  kindFilters: ReadonlySet<PatchKindFilter>;
  onToggleKind: (b: PatchKindFilter) => void;
  target: string;
  onTarget: (s: string) => void;
  showOlder: boolean;
  onToggleShowOlder: () => void;
  groupBy: GroupBy;
  onGroupBy: (g: GroupBy) => void;
  countVisible: number;
  countTotal: number;
  loading: boolean;
  onRefresh: () => void;
}

function Toolbar({
  kindFilters,
  onToggleKind,
  target,
  onTarget,
  showOlder,
  onToggleShowOlder,
  groupBy,
  onGroupBy,
  countVisible,
  countTotal,
  loading,
  onRefresh,
}: ToolbarProps) {
  return (
    <div className="border-border/60 bg-card/40 sticky top-0 z-10 flex flex-wrap items-center gap-2 border-b px-4 py-2 text-xs">
      <span className="font-mono text-muted-foreground">
        {loading ? "loading…" : `${countVisible} / ${countTotal}`}
      </span>
      <span className="mx-1 h-4 w-px bg-border" />
      {PATCH_KIND_FILTERS.map((f) => (
        <FilterChip
          key={f.id}
          label={f.label}
          title={f.description}
          active={kindFilters.has(f.id)}
          onClick={() => onToggleKind(f.id)}
        />
      ))}
      <span className="mx-1 h-4 w-px bg-border" />
      <Input
        value={target}
        onChange={(e) => onTarget(e.target.value)}
        placeholder="target file…"
        className="h-7 w-48 text-xs"
      />
      <span className="mx-1 h-4 w-px bg-border" />
      <FilterChip
        label={showOlder ? "all ages" : "<14d"}
        title={
          showOlder
            ? "Showing every queued patch including ones older than 14 days."
            : "Hiding patches older than 14 days (default). Click to include stale proposals."
        }
        active={!showOlder}
        onClick={onToggleShowOlder}
      />
      <span className="mx-1 h-4 w-px bg-border" />
      <span className="text-muted-foreground">group by</span>
      <FilterChip
        label="repo"
        title="Group by owning repo (default)."
        active={groupBy === "repo"}
        onClick={() => onGroupBy("repo")}
      />
      <FilterChip
        label="target"
        title="Group by target file path."
        active={groupBy === "target"}
        onClick={() => onGroupBy("target")}
      />
      <span className="ml-auto" />
      <Button
        variant="ghost"
        size="sm"
        className="h-7 px-2 text-xs"
        onClick={onRefresh}
        disabled={loading}
      >
        refresh
      </Button>
    </div>
  );
}

function FilterChip({
  label,
  title,
  active,
  onClick,
}: {
  label: string;
  title: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={cn(
        "rounded-md border px-2 py-0.5 font-mono text-[11px]",
        active
          ? "border-violet-500/40 bg-violet-500/15 text-violet-500"
          : "border-border/60 bg-card text-muted-foreground hover:text-foreground",
      )}
    >
      {label}
    </button>
  );
}

function GroupHeader({
  label,
  count,
  groupBy,
}: {
  label: string;
  count: number;
  groupBy: GroupBy;
}) {
  return (
    <div className="flex items-baseline gap-2 px-1 pt-2">
      <span className="text-xs uppercase tracking-wider text-muted-foreground">
        {groupBy === "repo" ? "repo" : "target"}
      </span>
      <span className="font-mono text-sm font-medium">{label}</span>
      <span className="font-mono text-[11px] text-muted-foreground">
        ({count})
      </span>
    </div>
  );
}

function EmptyState({ message, hint }: { message: string; hint: string }) {
  return (
    <div className="text-muted-foreground border-border/60 rounded-md border border-dashed px-4 py-6 text-center text-sm">
      <p>{message}</p>
      <p className="mt-1 text-xs">{hint}</p>
    </div>
  );
}

export type { ProposedPatchListEntry };
