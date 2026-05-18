// M4c exit test: covers the two shapes of the detach modal that
// DOCS/WORKSPACE-ATTACH.md §"Detach modal" pins.
//
//   * Happy path: no running jobs -> first click submits with the
//     default `refuse` policy and resolves; the row leaves the store.
//   * Running-jobs path: server replies with the structured
//     `WorkspaceError::RunningJobs` variant -> modal flips to the
//     two-radio choice and the per-job list renders inline; picking
//     `Stop them` resubmits with `stop` and resolves.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { AttachedWorkspace, RepoId } from "@/lib/rpc/wire";

import { DetachWorkspaceDialog } from "./DetachWorkspaceDialog";
import { useWorkspacesStore } from "./store";

function seedWorkspace(client: MockRpcClient): AttachedWorkspace {
  const ws: AttachedWorkspace = {
    repo_id: "repo-detach" as RepoId,
    repo_name: "alpha",
    fs_root: "/tmp/alpha",
    attached_at: 1_000,
    default_runner: null,
  };
  client.seedAttachedWorkspaces([ws]);
  useWorkspacesStore.setState({
    workspaces: [ws],
    activeRepoId: ws.repo_id,
    status: "ready",
    error: null,
  });
  return ws;
}

function mount(client: MockRpcClient, ws: AttachedWorkspace | null, onClose: () => void) {
  return render(
    <RpcProvider client={client}>
      <DetachWorkspaceDialog workspace={ws} onClose={onClose} />
    </RpcProvider>,
  );
}

beforeEach(() => {
  useWorkspacesStore.setState({
    workspaces: [],
    activeRepoId: null,
    status: "idle",
    error: null,
  });
});

afterEach(() => {
  cleanup();
});

describe("DetachWorkspaceDialog", () => {
  it("happy path: no running jobs — one-line confirm submits with refuse and detaches", async () => {
    const client = new MockRpcClient();
    const ws = seedWorkspace(client);
    let closed = 0;
    mount(client, ws, () => {
      closed += 1;
    });

    // No running-jobs state up front: the radio group must not render.
    expect(screen.queryByTestId("detach-policy-group")).not.toBeInTheDocument();
    expect(screen.queryByTestId("detach-running-jobs")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("detach-ws-submit-button"));

    await waitFor(() => expect(closed).toBe(1));
    expect(useWorkspacesStore.getState().workspaces).toEqual([]);
  });

  it("running jobs: refuse path surfaces RunningJobs inline; choosing Stop detaches", async () => {
    const client = new MockRpcClient();
    const ws = seedWorkspace(client);
    client.seedRunningJobsForWorkspace(ws.repo_id, ["job-a", "job-b"]);

    let closed = 0;
    mount(client, ws, () => {
      closed += 1;
    });

    fireEvent.click(screen.getByTestId("detach-ws-submit-button"));

    // The first click is the implicit `refuse`. The server rejects with
    // `WorkspaceError::RunningJobs`, the dialog parses the variant, and
    // the per-job list + radio render in-place — no toast, no string-
    // match on a generic Conflict.
    await waitFor(() =>
      expect(screen.getByTestId("detach-policy-group")).toBeInTheDocument(),
    );
    const jobsList = screen.getByTestId("detach-running-jobs");
    expect(jobsList).toHaveTextContent("job-a");
    expect(jobsList).toHaveTextContent("job-b");
    expect(closed).toBe(0);

    // `stop` is the default selection in the dialog; click submit
    // again and the mock collapses the seed + detaches.
    fireEvent.click(screen.getByTestId("detach-ws-submit-button"));
    await waitFor(() => expect(closed).toBe(1));
    expect(useWorkspacesStore.getState().workspaces).toEqual([]);
  });
});
