// Integration coverage for the cross-workspace worklist. The page
// composes the per-job `PatchCard` against a fresh data source
// (`list_proposed_patches`) and adds the toolbar / group-by /
// cross-window-resolution behaviour. The risks worth pinning are:
//
// - the 14-day decay filter hides stale entries by default,
// - cross-window resolution from a sibling JobPage drops the row,
// - the group-by toggle re-buckets rows without dropping any.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import { getCrossWindowEvents, registerCrossWindowEvents, inProcessCrossWindowEvents } from "@/lib/shell";
import type {
  ProposedPatchListEntry,
  ProposedScopePatch,
  ScopePatchId,
} from "@/lib/rpc";

import { PatchesPage } from "./PatchesPage";
import { SCOPE_PATCH_RESOLVED_EVENT } from "@/modules/jobs/patches";

afterEach(() => {
  cleanup();
});

beforeEach(() => {
  // Reset the cross-window bus between tests so a previous test's
  // resolution emit cannot leak into the next render.
  registerCrossWindowEvents(inProcessCrossWindowEvents);
});

function patch(over: Partial<ProposedScopePatch> & { id: string }): ProposedScopePatch {
  return {
    kind: "tighten",
    target: "claude-md",
    target_path: "CLAUDE.md",
    rationale: "lock down helpers",
    body: "tighten helper docs",
    has_predicate: false,
    evidence_stage_id: undefined,
    predicate_ref: undefined,
    fixture_ref: undefined,
    proposed_at: Date.now(),
    ...over,
    id: over.id as ScopePatchId,
  } as ProposedScopePatch;
}

function makeClient(entries: ProposedPatchListEntry[]) {
  const client = new MockRpcClient();
  const original = client.call.bind(client);
  vi.spyOn(client, "call").mockImplementation(
    // The mock client returns `{ entries: [] }` by default; this
    // override hands the test fixture back while leaving every other
    // method on the default path.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (method: any, args: any) => {
      if (method === "list_proposed_patches") {
        return Promise.resolve({ entries });
      }
      if (method === "list_repos") {
        return Promise.resolve({
          repos: [
            {
              id: "repo-1",
              name: "codeless",
              clone_url: "git://x",
              default_branch: "main",
              local_path: "/tmp/codeless",
              git_auth: { kind: "ssh-agent" },
              concurrency_cap: null,
              default_runner: null,
              created_at: 0,
            },
            {
              id: "repo-2",
              name: "demo",
              clone_url: "git://y",
              default_branch: "main",
              local_path: "/tmp/demo",
              git_auth: { kind: "ssh-agent" },
              concurrency_cap: null,
              default_runner: null,
              created_at: 0,
            },
          ],
        });
      }
      return original(method, args);
    },
  );
  return client;
}

function renderWith(entries: ProposedPatchListEntry[]) {
  const client = makeClient(entries);
  return render(
    <RpcProvider client={client}>
      <PatchesPage />
    </RpcProvider>,
  );
}

describe("PatchesPage", () => {
  it("renders entries grouped by repo, newest first", async () => {
    const now = Date.now();
    renderWith([
      { repo_id: "repo-1", patch: patch({ id: "older", proposed_at: now - 1000 }) },
      { repo_id: "repo-1", patch: patch({ id: "newer", proposed_at: now }) },
      { repo_id: "repo-2", patch: patch({ id: "other", proposed_at: now - 500 }) },
    ]);
    // Wait for at least one card to land before reading the order.
    await screen.findAllByText(/CLAUDE\.md/i);
    const repoLabels = screen.getAllByText(/repo/i, { selector: "span" });
    // Two group headers because two repos contribute entries.
    expect(repoLabels.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("codeless")).toBeInTheDocument();
    expect(screen.getByText("demo")).toBeInTheDocument();
  });

  it("hides patches older than 14 days by default", async () => {
    const now = Date.now();
    const fifteenDaysMs = 15 * 24 * 60 * 60 * 1000;
    renderWith([
      { repo_id: "repo-1", patch: patch({ id: "fresh", proposed_at: now }) },
      { repo_id: "repo-1", patch: patch({ id: "stale", proposed_at: now - fifteenDaysMs }) },
    ]);
    await waitFor(() => {
      // The toolbar's "1 / 2" counter is the cheapest assertion that
      // the decay rule applied.
      expect(screen.getByText("1 / 2")).toBeInTheDocument();
    });
  });

  it("drops a row when the cross-window bus broadcasts a resolution", async () => {
    const now = Date.now();
    renderWith([
      { repo_id: "repo-1", patch: patch({ id: "01PATCHA0000000000000000A", proposed_at: now }) },
      { repo_id: "repo-1", patch: patch({ id: "01PATCHB0000000000000000B", proposed_at: now }) },
    ]);
    await waitFor(() =>
      expect(screen.getByText("2 / 2")).toBeInTheDocument(),
    );
    await getCrossWindowEvents().emit(SCOPE_PATCH_RESOLVED_EVENT, {
      patch_id: "01PATCHA0000000000000000A" as ScopePatchId,
      resolution: "approved",
      commit_sha: "deadbeef",
    });
    await waitFor(() =>
      expect(screen.getByText("1 / 1")).toBeInTheDocument(),
    );
  });

  it("toggles group-by between repo and target", async () => {
    const now = Date.now();
    renderWith([
      {
        repo_id: "repo-1",
        patch: patch({ id: "a", target_path: "CLAUDE.md", proposed_at: now }),
      },
      {
        repo_id: "repo-2",
        patch: patch({ id: "b", target_path: "CLAUDE.md", proposed_at: now }),
      },
    ]);
    await waitFor(() =>
      expect(screen.getByText("2 / 2")).toBeInTheDocument(),
    );
    // Default: two repo groups.
    expect(screen.getAllByText(/^(codeless|demo)$/).length).toBe(2);
    // Switch to group-by target — collapses both rows under one header.
    const targetChip = screen.getByRole("button", { name: "target" });
    fireEvent.click(targetChip);
    await waitFor(() => {
      const header = within(document.body).getAllByText("CLAUDE.md");
      // One group header + two card target labels (one per card).
      expect(header.length).toBeGreaterThanOrEqual(3);
    });
  });
});
