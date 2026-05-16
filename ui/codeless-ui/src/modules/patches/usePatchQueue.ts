// Fetches the cross-workspace proposed-patch queue and keeps it in
// sync with cross-window approvals. Powers both the worklist page
// and the count badge in the global nav — colocated here so both
// surfaces agree on what counts as "still actionable."
//
// The hook does NOT subscribe to the SSE event stream: per-repo SSE
// filters do not exist on the workspace-level walk, and the per-job
// inbox already covers the live-add case. The worklist's cadence is
// "snapshot on mount + invalidate on cross-window resolution"; that
// matches the editor's mental model of opening `/patches` Monday
// morning and acting on what is there.

import { useCallback, useEffect, useState } from "react";

import { useRpc } from "@/lib/rpc";
import type {
  ProposedPatchListEntry,
  RepoId,
  ScopePatchId,
} from "@/lib/rpc";
import { getCrossWindowEvents } from "@/lib/shell";
import {
  SCOPE_PATCH_RESOLVED_EVENT,
  type ScopePatchResolvedPayload,
} from "@/modules/jobs/patches";

export interface UsePatchQueueResult {
  entries: ProposedPatchListEntry[] | null;
  error: Error | null;
  loading: boolean;
  // Manual refetch, exposed for the post-resolution path where the
  // worklist's own approve/reject completed inside this window. The
  // cross-window listener calls this for sibling-window resolutions;
  // the in-window approve handler calls it too so the row drops out
  // immediately rather than waiting for the bus to round-trip.
  refetch: () => void;
}

export function usePatchQueue(repoId: RepoId | null = null): UsePatchQueueResult {
  const rpc = useRpc();
  const [entries, setEntries] = useState<ProposedPatchListEntry[] | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);

  const refetch = useCallback(() => {
    setTick((t) => t + 1);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    rpc
      .call("list_proposed_patches", { repo_id: repoId ?? undefined })
      .then((r) => {
        if (cancelled) return;
        setEntries(r.entries);
        setError(null);
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setEntries(null);
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, repoId, tick]);

  // Cross-window invalidation. JobPage's PatchesTab broadcasts a
  // resolution payload after every approve / reject / revert; we
  // can drop the affected row from local state without another RPC
  // because the runtime's queue file has already removed the entry.
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      const unsub = await getCrossWindowEvents().listen<ScopePatchResolvedPayload>(
        SCOPE_PATCH_RESOLVED_EVENT,
        (payload) => {
          dropResolvedRow(payload.patch_id);
        },
      );
      if (cancelled) unsub();
      else dispose = unsub;
    })();
    return () => {
      cancelled = true;
      dispose?.();
    };
    // The setter is stable; intentionally not listed in deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function dropResolvedRow(patchId: ScopePatchId) {
    setEntries((curr) =>
      curr === null ? curr : curr.filter((e) => e.patch.id !== patchId),
    );
  }

  return { entries, error, loading, refetch };
}
