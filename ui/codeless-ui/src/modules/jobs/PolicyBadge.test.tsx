// PolicyBadge visibility + label. The badge is the operator's
// always-on reminder that the job is hands-off; if it renders for
// a null policy or labels a preset wrong, the operator loses the
// signal. The set_job_policy roundtrip itself is exercised through
// the mock client elsewhere — these tests are pure-render.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { AutoBypassPolicy, Job, JobStatus } from "@/lib/rpc/wire";

import { PolicyBadge, policyDisplayName } from "./PolicyBadge";

function makeJob(
  status: JobStatus,
  policy: AutoBypassPolicy | null,
): Job {
  return {
    id: "01HMOCKJOB000000000000000000",
    repo_id: "01HMOCKREPO0000000000000000",
    status,
    stop_reason: null,
    template_yaml: "name: smoke\n",
    prompt: "noop",
    runner: "mock",
    branch: "feat/smoke",
    workspace_mode: "in-repo",
    worktree_path: null,
    cost_cap_cents: 100,
    wall_clock_cap_ms: 60_000,
    cost_cents: 0,
    model: null,
    permission_mode: null,
    effort: null,
    system_prompt: null,
    persona_id: null,
    auto_bypass_policy: policy,
    started_at: null,
    ended_at: null,
    created_at: Date.now(),
  };
}

function renderBadge(job: Job) {
  const client = new MockRpcClient();
  return render(
    <RpcProvider client={client}>
      <PolicyBadge job={job} onUpdated={() => {}} />
    </RpcProvider>,
  );
}

describe("policyDisplayName", () => {
  it("names every preset variant", () => {
    expect(policyDisplayName({ type: "quick" })).toBe("Quick");
    expect(policyDisplayName({ type: "long-term" })).toBe("Long-term");
    expect(policyDisplayName({ type: "cheap" })).toBe("Cheap");
    expect(policyDisplayName({ type: "best-judgement" })).toBe("Best judgement");
    expect(policyDisplayName({ type: "just-code" })).toBe("Just code");
  });

  it("collapses custom comment under the Custom label", () => {
    expect(
      policyDisplayName({ type: "custom", comment: "ship it" }),
    ).toBe("Custom");
  });
});

describe("PolicyBadge", () => {
  afterEach(() => cleanup());

  it("renders nothing when the job has no policy", () => {
    const { container } = renderBadge(makeJob("draft", null));
    expect(container.textContent ?? "").not.toMatch(/policy:/i);
  });

  it("shows the preset label on a job with a policy", () => {
    renderBadge(makeJob("draft", { type: "quick" }));
    const btn = screen.getByRole("button", { name: /policy: quick/i });
    expect(btn).toBeInTheDocument();
  });

  it("titles the button as locked when the job is running", () => {
    renderBadge(makeJob("running", { type: "long-term" }));
    const btn = screen.getByRole("button", { name: /policy: long-term/i });
    // The lock signal lives on the title attribute so screen
    // readers and hover-tips both surface it.
    expect(btn.getAttribute("title")).toMatch(/locked|pause/i);
  });
});
