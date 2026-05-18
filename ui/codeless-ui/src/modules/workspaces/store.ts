// Per-tab active-workspace store. Mirrors the pattern used by
// `useChatStore` / `useInlineSettingsStore`: a single zustand singleton
// scoped to the JS realm (one per browser tab / desktop window). The
// server-side `attached_workspaces` table is the source of truth
// (R4) — this store is a live mirror that hydrates from
// `list_workspaces` and reconciles incrementally as
// `workspace_attached` / `workspace_detached` events arrive over the
// `RpcClient.subscribe()` channel.
//
// The "active" workspace is a UI projection: the server has no notion
// of which workspace this tab is currently viewing. On detach of the
// active workspace, the store falls back to the most-recently-attached
// remaining row (by `attached_at` descending) per the doc's
// §"Edge cases — Browser tab open against a detached workspace".

import { create } from "zustand";

import type { AttachedWorkspace, RepoId } from "@/lib/rpc/wire";

export type WorkspaceHydrationStatus = "idle" | "loading" | "ready" | "error";

interface WorkspacesState {
  workspaces: AttachedWorkspace[];
  activeRepoId: RepoId | null;
  status: WorkspaceHydrationStatus;
  error: string | null;

  // Replace the full list — used by the hydrate hook on initial
  // `list_workspaces` resolution and on transport reconnect when the
  // subscription replays from an old cursor.
  setWorkspaces(workspaces: AttachedWorkspace[]): void;

  // Mark loading / error so consumers can render a spinner or surface
  // a hydration failure without round-tripping through the RPC layer
  // again. The empty-state screen only renders when status is `ready`
  // and `workspaces.length === 0`.
  setStatus(status: WorkspaceHydrationStatus, error?: string | null): void;

  // Incremental reconciliation entry points. Idempotent: re-applying
  // the same `workspace_attached` event (e.g. after a transport
  // reconnect that replays from an old cursor) collapses on
  // `repo_id`, matching the server's unique index on the canonical
  // path.
  applyAttached(workspace: AttachedWorkspace): void;
  applyDetached(repoId: RepoId): void;

  // Active-workspace projection. `setActive(null)` clears it (used by
  // the empty-state surface and by the detach fallback when no
  // workspaces remain).
  setActive(repoId: RepoId | null): void;
}

function pickFallbackActive(
  rows: AttachedWorkspace[],
  removed: RepoId,
): RepoId | null {
  const remaining = rows.filter((w) => w.repo_id !== removed);
  if (remaining.length === 0) return null;
  // Most-recently-attached wins, ties broken by repo_id for
  // determinism in tests.
  const sorted = [...remaining].sort((a, b) => {
    if (a.attached_at !== b.attached_at) return b.attached_at - a.attached_at;
    return a.repo_id < b.repo_id ? -1 : 1;
  });
  return sorted[0]!.repo_id;
}

export const useWorkspacesStore = create<WorkspacesState>((set, get) => ({
  workspaces: [],
  activeRepoId: null,
  status: "idle",
  error: null,

  setWorkspaces: (workspaces) =>
    set((s) => {
      const existing = s.activeRepoId;
      const stillThere =
        existing !== null && workspaces.some((w) => w.repo_id === existing);
      let nextActive: RepoId | null = stillThere ? existing : null;
      if (nextActive === null && workspaces.length > 0) {
        // First hydrate (or active row vanished while we were offline)
        // — pick the most-recently-attached as the tab's default view.
        const sorted = [...workspaces].sort(
          (a, b) => b.attached_at - a.attached_at,
        );
        nextActive = sorted[0]!.repo_id;
      }
      return { workspaces, activeRepoId: nextActive };
    }),

  setStatus: (status, error = null) => set({ status, error }),

  applyAttached: (workspace) =>
    set((s) => {
      const without = s.workspaces.filter((w) => w.repo_id !== workspace.repo_id);
      const next = [...without, workspace];
      // First workspace ever attached for this tab becomes the active
      // view automatically; otherwise leave the user's current choice
      // alone — silent active-switching on every attach surprises the
      // user.
      const activeRepoId =
        s.activeRepoId === null ? workspace.repo_id : s.activeRepoId;
      return { workspaces: next, activeRepoId };
    }),

  applyDetached: (repoId) =>
    set((s) => {
      const next = s.workspaces.filter((w) => w.repo_id !== repoId);
      const activeRepoId =
        s.activeRepoId === repoId
          ? pickFallbackActive(s.workspaces, repoId)
          : s.activeRepoId;
      return { workspaces: next, activeRepoId };
    }),

  setActive: (repoId) => {
    if (repoId !== null) {
      const present = get().workspaces.some((w) => w.repo_id === repoId);
      if (!present) return;
    }
    set({ activeRepoId: repoId });
  },
}));

// Pure-function variant exposed for tests + the subscription
// reconciler: keeps the reducer logic testable without a React tree
// and without re-implementing the same fallback rules in two places.
export const __workspacesTestables = { pickFallbackActive };
