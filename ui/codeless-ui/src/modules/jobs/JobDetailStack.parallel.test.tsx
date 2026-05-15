// Regression coverage for the second-job-detail-tab blank-pane bug.
//
// JobDetailStack mounts every job-detail tab simultaneously and toggles
// visibility (load-bearing for "instant switch, no SSE tear-down").
// Two parallel JobPage instances must therefore behave independently:
// each must subscribe to its own job scope, each useJob query must
// resolve from its own jobId, and a newly-mounted JobPage must not
// inherit a sibling's `?tab=stage:...` URL hint.
//
// This test exercises a sequence the user can produce manually:
//   1. open one job tab (A); navigate to a stage row so the URL becomes
//      `?tab=stage:<A-only-stage>`,
//   2. open a second job tab (B) while A's stage URL is still live,
//   3. switch to B.
//
// On master, JobPage(B)'s `activeTab` lazy initialiser reads the shared
// `window.location.search` regardless of `active`, so B mounts pointing
// at A's stageId — B's Stages system tab is no longer aria-selected,
// and B's content area renders an empty StageDetail (the stage belongs
// to a different job). After the fix, both JobPages keep their own
// default Stages tab aria-selected.

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

    // Default tab per JOB-UI.md is Stages. Both JobPages must therefore
    // have their `Stages` system tab aria-selected. On master, JobPage
    // B's lazy initialiser reads `window.location.search` and adopts
    // A's `?tab=stage:A-only-stage`, so only ONE Stages tab in the DOM
    // is aria-selected (A's). After the fix this returns to TWO.
    const selectedStagesTabs = screen
      .getAllByRole("tab", { name: /Stages/, hidden: true })
      .filter((el) => el.getAttribute("aria-selected") === "true");
    expect(selectedStagesTabs).toHaveLength(2);
  });
});
