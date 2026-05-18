import { beforeEach, describe, expect, it } from "vitest";

import type { AttachedWorkspace, RepoId } from "@/lib/rpc/wire";

import { useWorkspacesStore } from "./store";
import { reconcileFromEvent } from "./useWorkspacesSync";

const ws = (
  id: string,
  name: string,
  attached_at: number,
): AttachedWorkspace => ({
  repo_id: id as RepoId,
  repo_name: name,
  fs_root: `/tmp/${name}`,
  attached_at,
  default_runner: null,
});

beforeEach(() => {
  useWorkspacesStore.setState({
    workspaces: [],
    activeRepoId: null,
    status: "idle",
    error: null,
  });
});

describe("useWorkspacesStore", () => {
  it("setWorkspaces seeds the list and elects the most-recent attach as active on first hydrate", () => {
    const a = ws("r-a", "alpha", 1_000);
    const b = ws("r-b", "beta", 2_000);
    useWorkspacesStore.getState().setWorkspaces([a, b]);
    const s = useWorkspacesStore.getState();
    expect(s.workspaces).toHaveLength(2);
    expect(s.activeRepoId).toBe("r-b");
  });

  it("setWorkspaces preserves the active workspace when it is still present", () => {
    const a = ws("r-a", "alpha", 1_000);
    const b = ws("r-b", "beta", 2_000);
    useWorkspacesStore.getState().setWorkspaces([a, b]);
    useWorkspacesStore.getState().setActive("r-a" as RepoId);
    // re-hydrate with the same set (e.g. transport reconnect replay)
    useWorkspacesStore.getState().setWorkspaces([a, b]);
    expect(useWorkspacesStore.getState().activeRepoId).toBe("r-a");
  });

  it("applyAttached upserts on repo_id and becomes active when the store was empty", () => {
    const a = ws("r-a", "alpha", 1_000);
    useWorkspacesStore.getState().applyAttached(a);
    expect(useWorkspacesStore.getState().workspaces).toHaveLength(1);
    expect(useWorkspacesStore.getState().activeRepoId).toBe("r-a");

    // Re-applying the same event after a replay is idempotent.
    useWorkspacesStore.getState().applyAttached(a);
    expect(useWorkspacesStore.getState().workspaces).toHaveLength(1);

    // Second attach does NOT silently steal active focus from the user.
    const b = ws("r-b", "beta", 3_000);
    useWorkspacesStore.getState().applyAttached(b);
    expect(useWorkspacesStore.getState().workspaces).toHaveLength(2);
    expect(useWorkspacesStore.getState().activeRepoId).toBe("r-a");
  });

  it("applyDetached drops the row and falls back to the most-recently-attached survivor when the active one is removed", () => {
    const a = ws("r-a", "alpha", 1_000);
    const b = ws("r-b", "beta", 2_000);
    const c = ws("r-c", "gamma", 3_000);
    useWorkspacesStore.getState().setWorkspaces([a, b, c]);
    useWorkspacesStore.getState().setActive("r-c" as RepoId);
    useWorkspacesStore.getState().applyDetached("r-c" as RepoId);
    const s = useWorkspacesStore.getState();
    expect(s.workspaces.map((w) => w.repo_id)).toEqual(["r-a", "r-b"]);
    expect(s.activeRepoId).toBe("r-b");
  });

  it("applyDetached clears active when no workspaces remain", () => {
    const a = ws("r-a", "alpha", 1_000);
    useWorkspacesStore.getState().setWorkspaces([a]);
    useWorkspacesStore.getState().applyDetached("r-a" as RepoId);
    const s = useWorkspacesStore.getState();
    expect(s.workspaces).toEqual([]);
    expect(s.activeRepoId).toBeNull();
  });

  it("setActive ignores a repo_id that is not in the roster", () => {
    const a = ws("r-a", "alpha", 1_000);
    useWorkspacesStore.getState().setWorkspaces([a]);
    useWorkspacesStore.getState().setActive("r-ghost" as RepoId);
    expect(useWorkspacesStore.getState().activeRepoId).toBe("r-a");
  });
});

describe("reconcileFromEvent", () => {
  it("dispatches workspace-attached payloads into the store", () => {
    const a = ws("r-a", "alpha", 1_000);
    reconcileFromEvent({ type: "workspace-attached", workspace: a });
    expect(useWorkspacesStore.getState().workspaces).toHaveLength(1);
  });

  it("dispatches workspace-detached payloads into the store", () => {
    const a = ws("r-a", "alpha", 1_000);
    useWorkspacesStore.getState().setWorkspaces([a]);
    reconcileFromEvent({ type: "workspace-detached", repo_id: "r-a" });
    expect(useWorkspacesStore.getState().workspaces).toEqual([]);
  });

  it("ignores unrelated event types", () => {
    const a = ws("r-a", "alpha", 1_000);
    useWorkspacesStore.getState().setWorkspaces([a]);
    reconcileFromEvent({ type: "job-completed", job_id: "j-1" });
    expect(useWorkspacesStore.getState().workspaces).toHaveLength(1);
  });
});
