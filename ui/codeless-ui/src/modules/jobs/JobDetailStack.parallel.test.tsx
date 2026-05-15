// Regression coverage for the second-job-detail-tab blank-pane bug.
//
// JobDetailStack mounts every job-detail tab simultaneously and toggles
// visibility (load-bearing for "instant switch, no SSE tear-down").
// The bug: each JobPage was wrapped in `<div class="h-full w-full">`
// with no positioning. Two such wrappers stack vertically inside their
// `absolute inset-0` parent — the first claims the full viewport, the
// second is pushed entirely below it. The active JobPage renders fine
// in the DOM but is not visible. AiDiffStack / EditorStack /
// PreviewStack avoid this by positioning each child wrapper with
// `absolute inset-0` so they overlap.

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { EventFilter, Since } from "@/lib/rpc/methods";
import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { EventEnvelope, Job } from "@/lib/rpc/wire";
import type { JobDetailTab } from "@/modules/tabs";

import { JobDetailStack } from "./JobDetailStack";

class RecordingMock extends MockRpcClient {
  public observedFilters: EventFilter[] = [];

  // Capture every `subscribe()` call so the test can assert each
  // JobPage opened its own per-jobId scope. Delegates the actual
  // subscription work to the base mock unchanged.
  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope> {
    this.observedFilters.push(filter);
    return super.subscribe(filter, since);
  }
}

async function makeDraftJob(client: MockRpcClient, name: string): Promise<Job> {
  const repos = await client.call("list_repos", {});
  const repo = repos.repos[0];
  const yaml = `name: ${name}\ngoal: smoke test\nstages:\n  - id: s1\n    name: plan\n`;
  return client.call("submit_job", {
    repo_id: repo.id,
    template_yaml: yaml,
    prompt: "test",
    runner: "mock",
    branch: `feat/${name}`,
    workspace_mode: "in-repo",
    cost_cap_cents: 100,
    wall_clock_cap_ms: 60_000,
    start_immediately: false,
  });
}

function buildTabs(
  jobA: Job,
  jobB: Job | null,
): Array<JobDetailTab> {
  const tabs: JobDetailTab[] = [
    { id: 1, kind: "job-detail", title: "alpha", jobId: jobA.id },
  ];
  if (jobB) {
    tabs.push({ id: 2, kind: "job-detail", title: "bravo", jobId: jobB.id });
  }
  return tabs;
}

describe("JobDetailStack with two parallel JobPage instances", () => {
  afterEach(() => {
    cleanup();
    window.history.replaceState(null, "", "/jobs");
  });

  it("keeps each JobPage's state and subscription scoped to its own jobId", async () => {
    window.history.replaceState(null, "", "/jobs");

    const client = new RecordingMock();
    const jobA = await makeDraftJob(client, "alpha-job");
    const jobB = await makeDraftJob(client, "bravo-job");

    // Step 1: only the first tab is open and active.
    const { rerender } = render(
      <RpcProvider client={client}>
        <JobDetailStack tabs={buildTabs(jobA, null)} activeId={1} />
      </RpcProvider>,
    );

    // useJob(A) resolves from its own row.
    await waitFor(() => {
      expect(screen.getByText("alpha-job")).toBeInTheDocument();
    });

    // Step 2: the user clicks a stage row in A; in production the
    // JobPage's URL-mirror effect would write this. The test sets it
    // directly to keep the assertion focused on the mount-time read of
    // `window.location` by the *second* JobPage.
    window.history.replaceState(null, "", "/jobs?tab=stage:A-only-stage");

    // Step 3: open the second job tab while A's URL is still live, then
    // make B the active tab. Both JobPages are now mounted at once.
    rerender(
      <RpcProvider client={client}>
        <JobDetailStack tabs={buildTabs(jobA, jobB)} activeId={2} />
      </RpcProvider>,
    );

    // useJob(B) resolves from its own row, independently of A.
    await waitFor(() => {
      expect(screen.getByText("bravo-job")).toBeInTheDocument();
    });
    // A's title is still in the DOM (its JobPage is hidden, not torn down).
    expect(screen.getByText("alpha-job")).toBeInTheDocument();

    // Both JobPages opened their own per-job subscription. Multiple
    // subscribe() calls per job are expected (page-level event stream,
    // JobTabs indicator stream, StagesOverview stream) — what matters
    // is that A's scope and B's scope are both represented.
    const jobScopes = client.observedFilters
      .filter(
        (f): f is Extract<EventFilter, { scope: "job" }> => f.scope === "job",
      )
      .map((f) => f.job_id);
    expect(jobScopes).toContain(jobA.id);
    expect(jobScopes).toContain(jobB.id);

    // Both JobPage wrappers must share a positioned parent and stack via
    // `absolute inset-0` so they overlap. The unfixed implementation
    // wrapped each JobPage in `<div class="h-full w-full">` with no
    // positioning — the second wrapper rendered below the viewport in
    // normal flow. Walk up from each title element until we find the
    // common ancestor's direct children, then assert the layout contract.
    const titleA = screen.getByText("alpha-job");
    const titleB = screen.getByText("bravo-job");

    // The wrappers are the topmost <div>s rendered by JobDetailStack
    // for each tab. Find them by walking up to the nearest ancestor
    // whose parent contains both title elements.
    function findStackItem(el: HTMLElement): HTMLElement | null {
      let cur: HTMLElement | null = el;
      while (cur && cur.parentElement) {
        const parent: HTMLElement = cur.parentElement;
        if (parent.contains(titleA) && parent.contains(titleB)) {
          return cur;
        }
        cur = parent;
      }
      return null;
    }
    const wrapperA = findStackItem(titleA);
    const wrapperB = findStackItem(titleB);
    expect(wrapperA).not.toBeNull();
    expect(wrapperB).not.toBeNull();
    expect(wrapperA?.parentElement).toBe(wrapperB?.parentElement);
    // Both wrappers must be absolutely positioned so they overlap; the
    // shared parent must establish a positioning context. Without these,
    // the second wrapper renders below the viewport in normal flow.
    expect(wrapperA?.className).toMatch(/\babsolute\b/);
    expect(wrapperB?.className).toMatch(/\babsolute\b/);
    expect(wrapperA?.parentElement?.className).toMatch(/\brelative\b/);
  });
});
