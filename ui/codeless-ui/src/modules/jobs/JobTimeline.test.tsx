// Surface F coverage for the per-job timeline. Two render contracts
// pinned here, each load-bearing for the operator's ability to tell
// "the runtime recovered" apart from "the runtime halted":
//
//   1. A `stage-auto-bypassed` envelope renders a dedicated chip in
//      the timeline (the down-arrow-with-check glyph, not the generic
//      event row), and the chip's tooltip carries the prior stage's
//      `failure_class` + `failure_detail` as threaded through
//      `comment_used` by `auto_bypass_policy::policy_comment`. The
//      threaded text is what makes the chip a recovery story rather
//      than a bare "policy fired" label.
//
//   2. A hard failure with no bypass — `verify-failed` /
//      `stage-completed{failed}` without a following
//      `stage-auto-bypassed` envelope — renders the failure as a
//      plain event row and never produces the bypass chip.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { Event } from "@/lib/rpc/wire";

import { JobTimeline } from "./JobTimeline";

const JOB = "01HJOB0000000000000000000000";
const STAGE = "01HSTAGE0000000000000000000";
const TASK = "01HTASK00000000000000000000";

afterEach(() => cleanup());

// Reproduces the `policy_comment` output for the Quick preset when a
// `PriorFailure` is threaded in: a canned guidance paragraph followed
// by a fenced `Previous-stage failure: <wire-name>` + `Detail: …`
// block. The runtime stamps this string onto `comment_used` so the
// audit trail and the next stage's prompt see the same text; the
// timeline's tooltip reuses it verbatim.
const QUICK_THREADED_COMMENT =
  "Recover quickly and keep moving. Prefer the smallest fix that " +
  "restores progress.\n\n" +
  "```\n" +
  "Previous-stage failure: pre-check-failed\n" +
  "Detail: scope-patch path drift: src/auth/mod.rs\n" +
  "```";

describe("JobTimeline — auto-bypass chip", () => {
  it("renders the bypass chip with a tooltip threading failure_class + failure_detail", async () => {
    const client = new MockRpcClient();
    render(
      <RpcProvider client={client}>
        <JobTimeline jobId={JOB} />
      </RpcProvider>,
    );

    const emit = (
      ev: Event,
      stageId: string | null = STAGE,
      taskId: string | null = null,
    ) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (client as any).emit(ev, JOB, stageId, taskId);
    };

    emit({
      type: "stage-started",
      stage_id: STAGE,
      job_id: JOB,
      name: "auth",
      ordinal: 0,
    });
    emit({
      type: "stage-completed",
      stage_id: STAGE,
      status: "failed",
      failure_class: "pre-check-failed",
      failure_detail: "scope-patch path drift: src/auth/mod.rs",
    });
    emit({
      type: "stage-auto-bypassed",
      stage_id: STAGE,
      policy_name: "Quick",
      comment_used: QUICK_THREADED_COMMENT,
      applied_at: 1_700_000_001_000,
    });

    // The chip renders as a row carrying the canonical
    // "auto-bypassed" label + the policy name. The aria-label on the
    // glyph is the screen-reader hook for the recovery state, so a
    // future refactor that re-themes the glyph still has to keep it.
    await screen.findByText("auto-bypassed");
    expect(screen.getByLabelText("auto-bypassed")).toBeTruthy();
    expect(screen.getByText(/by policy: Quick/)).toBeInTheDocument();

    // The tooltip is `${policy_name}: ${comment_used}` and
    // `comment_used` is the policy comment already threaded with the
    // prior stage's failure_class + failure_detail by the runtime.
    // Assert both halves survive — the wire-name class label and the
    // verbatim detail string — so a future tooltip composer refactor
    // cannot silently drop either.
    const row = screen.getByText("auto-bypassed").closest("li");
    expect(row).not.toBeNull();
    const tip = row!.getAttribute("title") ?? "";
    expect(tip).toContain("Quick:");
    expect(tip).toContain("Previous-stage failure: pre-check-failed");
    expect(tip).toContain(
      "Detail: scope-patch path drift: src/auth/mod.rs",
    );
  });

  it("renders no bypass chip on a hard fail with no auto-bypass envelope", async () => {
    const client = new MockRpcClient();
    render(
      <RpcProvider client={client}>
        <JobTimeline jobId={JOB} />
      </RpcProvider>,
    );

    const emit = (
      ev: Event,
      stageId: string | null = STAGE,
      taskId: string | null = TASK,
    ) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (client as any).emit(ev, JOB, stageId, taskId);
    };

    emit({
      type: "stage-started",
      stage_id: STAGE,
      job_id: JOB,
      name: "auth",
      ordinal: 0,
    });
    emit({
      type: "task-enqueued",
      task_id: TASK,
      stage_id: STAGE,
      depends_on: [],
    });
    emit({ type: "task-started", task_id: TASK });
    // The failure row is the generic event-row render path; the
    // payload line shows the exit code from the runner. The chip-only
    // bypass branch is keyed off `stage-auto-bypassed`, which is
    // never emitted here.
    emit({ type: "verify-failed", stage_id: STAGE, exit_code: 1 });
    emit({
      type: "stage-completed",
      stage_id: STAGE,
      status: "failed",
      failure_class: "review-fail",
      failure_detail: "compile error in src/auth/mod.rs",
    });

    await screen.findByText("verify-failed");
    expect(screen.getByText("stage-completed")).toBeInTheDocument();
    // No bypass chip and no aria-label "auto-bypassed" anywhere — the
    // operator must see this as a halt, not a recovery.
    expect(screen.queryByText("auto-bypassed")).toBeNull();
    expect(screen.queryByLabelText("auto-bypassed")).toBeNull();
  });
});
