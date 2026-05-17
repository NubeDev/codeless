// Reset-button visibility for the recovery-hatch (`reset_job`) action.
//
// The Rust-side state machine accepts the `Queued | Failed | Stopped`
// -> `Draft` edge only — any other status is a server-side `Conflict`.
// The UI mirrors that contract: the button surfaces only when the
// runtime would accept the call. It must never be visible while a job
// is `Running` or `AwaitingReview`; otherwise an operator clicking it
// would race the still-driving runner. Other non-resettable statuses
// (`Draft`, `Paused`, `Completed`) are covered for symmetry.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { Job, JobStatus } from "@/lib/rpc/wire";

import { PageHeader } from "./JobPage";

function makeJob(status: JobStatus): Job {
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
    auto_bypass_policy: null,
    pending_operator_comment: null,
    started_at: null,
    ended_at: null,
    created_at: Date.now(),
  };
}

function renderHeader(job: Job) {
  const client = new MockRpcClient();
  return render(
    <RpcProvider client={client}>
      <PageHeader
        job={job}
        repoName="codeless"
        now={Date.now()}
        sseStatus={{ state: "live", since_ms: Date.now(), last_cursor: null }}
        title="smoke"
        refetchJob={() => {}}
      />
    </RpcProvider>,
  );
}

describe("PageHeader reset button gating", () => {
  afterEach(() => cleanup());

  for (const status of ["queued", "failed", "stopped"] as const) {
    it(`is visible when status=${status}`, () => {
      renderHeader(makeJob(status));
      expect(screen.getByRole("button", { name: /reset/i })).toBeInTheDocument();
    });
  }

  // The load-bearing invariant: the recovery hatch must not race a
  // live driver. Both `running` and `awaiting-review` count as "the
  // runner is in charge" for this purpose.
  for (const status of ["running", "awaiting-review"] as const) {
    it(`is hidden when status=${status}`, () => {
      renderHeader(makeJob(status));
      expect(screen.queryByRole("button", { name: /reset/i })).toBeNull();
    });
  }

  // Symmetry: Draft already is the reset target, Paused has resume,
  // Completed has rerun — none should see a Reset button.
  for (const status of ["draft", "paused", "completed"] as const) {
    it(`is hidden when status=${status}`, () => {
      renderHeader(makeJob(status));
      expect(screen.queryByRole("button", { name: /reset/i })).toBeNull();
    });
  }
});
