import { useCallback, useEffect, useMemo, useReducer, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import {
  useEventStream,
  useJob,
  useRpc,
  type EventEnvelope,
  type JobId,
  type RepoId,
  type ScopePatchId,
  type ScopePatchKind,
  type ScopePatchTarget,
  type StageId,
} from "@/lib/rpc";
import { getCrossWindowEvents } from "@/lib/shell";

import { PatchCard } from "./PatchCard";
import { ApproveDiffDialog } from "./ApproveDiffDialog";
import { UndoToast } from "./UndoToast";
import {
  renderProposalMarkdown,
  type PatchProposal,
  type PatchResolution,
} from "./proposal";

// Surface B from `DOCS/SCOPE-MUTABLE-UI.md` — the per-job patch
// inbox. Renders one card per `ScopePatchProposed` event the job has
// emitted, ordered by emit time. Resolution events arriving on the
// same SSE stream (or via the cross-window bus from a sibling window
// approving the same proposal) collapse the card into a "<resolved>"
// row that links to the resulting commit.
//
// The tab is hidden by JobPage when `proposalCount === 0`; this
// component renders no empty state of its own, because by the time
// it mounts there is at least one card to show.

// Cross-window event name fan-out for resolutions. Subscribed from
// `/patches` (Surface C, Stage 9) and from any other JobPage that
// happens to be showing the same proposal. The payload is the
// resolution event in normalised form so the listener can update
// its in-memory state without a follow-up RPC.
export const SCOPE_PATCH_RESOLVED_EVENT = "codeless://scope-patch-resolved";

export interface ScopePatchResolvedPayload {
  patch_id: ScopePatchId;
  resolution: "approved" | "rejected" | "reverted";
  commit_sha: string;
}

// One inbox row keyed by patch_id. `resolution` is `null` while the
// proposal is still actionable; non-null after an approve / reject
// either fires here or arrives from a sibling window. `proposedAt`
// is the SSE event's `created_at` — used for the "Proposed: <time>"
// metadata line in the card.
interface PatchRow {
  proposal: PatchProposal;
  proposedAt: number;
  resolution: PatchResolution | null;
}

type State = {
  // Ordered by emit time (oldest first); the card list reverses for
  // display so newest-first comes "without inventing a sort".
  rows: PatchRow[];
};

type Action =
  | { kind: "proposed"; row: PatchRow }
  | { kind: "resolved"; patchId: ScopePatchId; resolution: PatchResolution }
  | { kind: "edited"; patchId: ScopePatchId; proposal: PatchProposal };

function reducer(state: State, action: Action): State {
  switch (action.kind) {
    case "proposed": {
      // Dedup: SSE replay after reconnect must not double-add.
      const has = state.rows.some(
        (r) => r.proposal.id === action.row.proposal.id,
      );
      if (has) return state;
      return { rows: [...state.rows, action.row] };
    }
    case "resolved":
      return {
        rows: state.rows.map((r) =>
          r.proposal.id === action.patchId
            ? { ...r, resolution: action.resolution }
            : r,
        ),
      };
    case "edited":
      return {
        rows: state.rows.map((r) =>
          r.proposal.id === action.patchId
            ? { ...r, proposal: action.proposal }
            : r,
        ),
      };
  }
}

// Lift a `scope-patch-proposed` SSE event into a `PatchProposal`. The
// event carries every field the inbox renders today; the proposal's
// `body` and `rationale` are filled in lazily via Edit (which uses
// the `Proposal::render` round-trip; the SSE envelope does not carry
// them). Stage 9's `list_proposed_patches` RPC will replace this with
// a full parse from the queue file.
function proposalFromEvent(env: EventEnvelope): PatchProposal | null {
  const e = env.event;
  if (e.type !== "scope-patch-proposed") return null;
  return {
    id: e.patch_id,
    review_id: e.review_id,
    stage_id: e.stage_id,
    kind: e.kind,
    target: e.target,
    target_path: e.target_path,
    evidence_stage_id: e.evidence_stage_id,
    has_predicate: e.has_predicate,
    rationale: "",
    body: "",
  };
}

// Active toast state. The Approve flow opens a 10-second toast with
// the commit sha and a one-click revert; the toast clears when the
// timer fires, when the user clicks dismiss, or when they click
// Undo.
interface ToastState {
  patchId: ScopePatchId;
  commitSha: string;
  expiresAt: number;
}

interface Props {
  jobId: JobId;
}

export function PatchesTab({ jobId }: Props) {
  const rpc = useRpc();
  const { data: job } = useJob(jobId);
  const [state, dispatch] = useReducer(reducer, { rows: [] });
  const [toast, setToast] = useState<ToastState | null>(null);
  const [approveDiff, setApproveDiff] = useState<{
    patchId: ScopePatchId;
    original: string;
    edited: string;
  } | null>(null);

  // Subscribe to the job's SSE stream for proposal / resolution
  // events. Approval / rejection events emitted by either this window
  // or a sibling will arrive here; the cross-window fan-out (below)
  // covers /patches and other JobPage windows that subscribe to a
  // different SSE scope.
  const onEvent = useCallback((env: EventEnvelope) => {
    const e = env.event;
    if (e.type === "scope-patch-proposed") {
      const proposal = proposalFromEvent(env);
      if (proposal) {
        dispatch({
          kind: "proposed",
          row: { proposal, proposedAt: env.created_at, resolution: null },
        });
      }
    } else if (e.type === "scope-patch-approved") {
      dispatch({
        kind: "resolved",
        patchId: e.patch_id,
        resolution: { kind: "approved", commit_sha: e.commit_sha },
      });
    } else if (e.type === "scope-patch-rejected") {
      dispatch({
        kind: "resolved",
        patchId: e.patch_id,
        resolution: { kind: "rejected", commit_sha: e.commit_sha },
      });
    }
  }, []);
  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  // Listen to cross-window resolutions so an approve in JobPage
  // window A invalidates the inbox in window B without a follow-up
  // RPC. The handler is idempotent: applying a resolution to a row
  // that already has it is a no-op (reducer keeps the existing
  // value).
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      const unsub = await getCrossWindowEvents().listen<ScopePatchResolvedPayload>(
        SCOPE_PATCH_RESOLVED_EVENT,
        (payload) => {
          if (payload.resolution === "approved") {
            dispatch({
              kind: "resolved",
              patchId: payload.patch_id,
              resolution: { kind: "approved", commit_sha: payload.commit_sha },
            });
          } else if (payload.resolution === "rejected") {
            dispatch({
              kind: "resolved",
              patchId: payload.patch_id,
              resolution: { kind: "rejected", commit_sha: payload.commit_sha },
            });
          }
        },
      );
      if (cancelled) {
        unsub();
      } else {
        dispose = unsub;
      }
    })();
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  // Expire the undo toast 10s after it appears. Decision OQ#3 pins
  // the lifetime at "~10s"; we use exactly 10s so the keystroke
  // budget is predictable.
  useEffect(() => {
    if (!toast) return;
    const remaining = toast.expiresAt - Date.now();
    if (remaining <= 0) {
      setToast(null);
      return;
    }
    const id = setTimeout(() => setToast(null), remaining);
    return () => clearTimeout(id);
  }, [toast]);

  const broadcastResolved = useCallback(
    (payload: ScopePatchResolvedPayload) => {
      void getCrossWindowEvents().emit(SCOPE_PATCH_RESOLVED_EVENT, payload);
    },
    [],
  );

  const handleApprove = useCallback(
    async (patchId: ScopePatchId, opts?: { editedBody?: string }) => {
      if (!job) return;
      // Edit path: re-parse and replace the queue entry first so the
      // approval commit picks up the edited body. The runtime's
      // edit_scope_patch validates the rendered buffer before
      // touching the queue file, so an unparseable edit surfaces
      // before any commit happens.
      if (opts?.editedBody !== undefined) {
        const editRes = await rpc.call("edit_scope_patch", {
          repo_id: job.repo_id,
          patch_id: patchId,
          rendered: opts.editedBody,
        });
        if (
          editRes.outcome === "already_resolved" &&
          editRes.resolution !== undefined
        ) {
          dispatch({
            kind: "resolved",
            patchId,
            resolution: { kind: editRes.resolution, commit_sha: editRes.commit_sha ?? "" },
          });
          return;
        }
      }
      const result = await rpc.call("approve_scope_patch", {
        repo_id: job.repo_id,
        patch_id: patchId,
      });
      if (result.outcome === "approved") {
        dispatch({
          kind: "resolved",
          patchId,
          resolution: { kind: "approved", commit_sha: result.commit_sha },
        });
        // Plain-approve toast: the diff-modal path already gave the
        // operator a friction point to review their edit, so the
        // undo affordance is only useful from the as-is path. Per
        // decision OQ#3 the toast lives ~10s.
        if (opts?.editedBody === undefined) {
          setToast({
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
      } else if (result.outcome === "already_resolved") {
        dispatch({
          kind: "resolved",
          patchId,
          resolution: { kind: result.resolution, commit_sha: result.commit_sha ?? "" },
        });
      }
    },
    [job, rpc, broadcastResolved],
  );

  const handleReject = useCallback(
    async (patchId: ScopePatchId) => {
      if (!job) return;
      const result = await rpc.call("reject_scope_patch", {
        repo_id: job.repo_id,
        patch_id: patchId,
      });
      if (result.outcome === "rejected") {
        dispatch({
          kind: "resolved",
          patchId,
          resolution: { kind: "rejected", commit_sha: result.commit_sha },
        });
        broadcastResolved({
          patch_id: patchId,
          resolution: "rejected",
          commit_sha: result.commit_sha,
        });
      } else if (result.outcome === "already_resolved") {
        dispatch({
          kind: "resolved",
          patchId,
          resolution: { kind: result.resolution, commit_sha: result.commit_sha ?? "" },
        });
      }
    },
    [job, rpc, broadcastResolved],
  );

  // Approve-after-Edit: open the diff dialog with the original
  // proposal text vs the edited buffer so the operator can confirm
  // the delta before the approval commit lands. The dialog calls
  // `handleApprove` with `editedBody` set on confirm.
  const handleApproveAfterEdit = useCallback(
    (patchId: ScopePatchId, original: string, edited: string) => {
      setApproveDiff({ patchId, original, edited });
    },
    [],
  );

  const handleApproveDiffConfirm = useCallback(async () => {
    if (!approveDiff) return;
    const { patchId, edited } = approveDiff;
    setApproveDiff(null);
    await handleApprove(patchId, { editedBody: edited });
  }, [approveDiff, handleApprove]);

  const handleEditSaved = useCallback(
    (patchId: ScopePatchId, proposal: PatchProposal) => {
      // The local card surfaces the edited rationale / body
      // immediately; the queue file on disk is rewritten by the
      // approve-after-edit path when the operator clicks Approve in
      // the dialog. The Edit save is a UI-only checkpoint of the
      // edited buffer until then.
      dispatch({ kind: "edited", patchId, proposal });
    },
    [],
  );

  const handleUndo = useCallback(async () => {
    if (!job || !toast) return;
    const { patchId, commitSha } = toast;
    setToast(null);
    const result = await rpc.call("revert_scope_patch", {
      repo_id: job.repo_id,
      commit_sha: commitSha,
    });
    // After revert the inbox no longer treats the patch as actionable;
    // the proposal queue still has the entry removed (the approval
    // commit was preserved, just inverted). Surface this as
    // resolution = reverted so the row shows the right state.
    dispatch({
      kind: "resolved",
      patchId,
      resolution: { kind: "reverted", commit_sha: result.commit_sha },
    });
    broadcastResolved({
      patch_id: patchId,
      resolution: "reverted",
      commit_sha: result.commit_sha,
    });
  }, [job, rpc, toast, broadcastResolved]);

  // Cards render newest-first because the editor's most recent
  // attention is usually the most recent REVIEW stage. The dedup
  // reducer keeps the SSE-arrival order; the display reverse is
  // cheap because the count is bounded by REVIEW gates in one job.
  const cards = useMemo(
    () => [...state.rows].reverse(),
    [state.rows],
  );

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <ScrollArea className="flex-1">
        <div className="mx-auto flex max-w-3xl flex-col gap-3 px-4 py-4">
          {cards.map((row) => (
            <PatchCard
              key={row.proposal.id}
              proposal={row.proposal}
              proposedAt={row.proposedAt}
              resolution={row.resolution}
              onApprove={() => handleApprove(row.proposal.id)}
              onReject={() => handleReject(row.proposal.id)}
              onApproveAfterEdit={(edited) =>
                handleApproveAfterEdit(
                  row.proposal.id,
                  renderProposalMarkdown(row.proposal),
                  edited,
                )
              }
              onEditSaved={(updated) => handleEditSaved(row.proposal.id, updated)}
            />
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

// Re-export for tests that count proposals from the same SSE stream
// the JobPage uses to decide whether to render the tab.
export function isScopePatchProposed(
  env: EventEnvelope,
): env is EventEnvelope & {
  event: {
    type: "scope-patch-proposed";
    patch_id: ScopePatchId;
    review_id: StageId;
    stage_id: StageId;
    kind: ScopePatchKind;
    target: ScopePatchTarget;
    target_path: string;
    evidence_stage_id: StageId | null;
    has_predicate: boolean;
  };
} {
  return env.event.type === "scope-patch-proposed";
}

// Surface in JobPage: pass the job's `repo_id` if the tab needs it
// outside of `useJob()`. Currently `PatchesTab` resolves it via
// `useJob(jobId)` directly to keep the component self-contained.
export type { RepoId };
