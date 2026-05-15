// Render states for the Surface A summary panel. WORKFLOW.md item 3
// requires component coverage for the three load-bearing render states
// (pre-check pass with verified paths, pre-check fail with misses,
// auto-fail verdict). The fourth axis — patch counter row visibility
// gated on the runtime feature flag — is covered alongside, because
// shipping a counter that lies before Dep #2 lands is the OQ#1
// stopping-point invariant.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ReviewGatePanel } from "./ReviewGatePanel";

afterEach(() => cleanup());

describe("ReviewGatePanel", () => {
  it("renders pre-check pass with the verified path list", () => {
    render(
      <ReviewGatePanel
        precheck={{ outcome: "pass", verified: ["DOCS/SCOPE.md", "src/lib.rs"] }}
        verdict={{ verdict: "pass", reason: "all evidence matched" }}
        patchesProposed={0}
        patchCounterEnabled={false}
      />,
    );
    expect(screen.getByText(/verified 2 paths/)).toBeInTheDocument();
    expect(screen.getByText("DOCS/SCOPE.md")).toBeInTheDocument();
    expect(screen.getByText("src/lib.rs")).toBeInTheDocument();
    expect(screen.getByText("PASS")).toBeInTheDocument();
    expect(screen.getByText(/all evidence matched/)).toBeInTheDocument();
  });

  it("renders pre-check fail with the missing path list", () => {
    render(
      <ReviewGatePanel
        precheck={{ outcome: "fail", missing: ["DOCS/SCOPE.md"] }}
        verdict={{ verdict: "fail", reason: "claimed path absent from diff" }}
        patchesProposed={0}
        patchCounterEnabled={false}
      />,
    );
    expect(screen.getByText(/missing 1 path\b/)).toBeInTheDocument();
    expect(screen.getByText("DOCS/SCOPE.md")).toBeInTheDocument();
    expect(screen.getByText("FAIL")).toBeInTheDocument();
    expect(screen.getByText(/claimed path absent from diff/)).toBeInTheDocument();
  });

  it("renders auto-fail verdict with reason text", () => {
    render(
      <ReviewGatePanel
        precheck={{ outcome: "skipped" }}
        verdict={{
          verdict: "auto-fail",
          reason: "sentinel missing from handover",
        }}
        patchesProposed={0}
        patchCounterEnabled={false}
      />,
    );
    expect(screen.getByText("AUTO-FAIL")).toBeInTheDocument();
    expect(screen.getByText(/sentinel missing from handover/)).toBeInTheDocument();
    expect(screen.getByText(/skipped/)).toBeInTheDocument();
  });

  // OQ#1: the counter row is OMITTED, not zeroed, until the runtime
  // advertises the SCOPE-PATCH handover round-trip capability. A
  // counter that reads "0 proposed" when the runtime cannot actually
  // observe a proposal in raw_tail is worse than no counter, because
  // the editor reads it as "the gate ran and produced no patches"
  // rather than "the platform cannot see patches yet."
  it("omits the patches row when the runtime feature flag is off", () => {
    render(
      <ReviewGatePanel
        precheck={{ outcome: "pass", verified: [] }}
        verdict={{ verdict: "pass", reason: "" }}
        patchesProposed={3}
        patchCounterEnabled={false}
      />,
    );
    expect(screen.queryByText(/proposed/)).toBeNull();
  });

  it("shows the patches row only when the feature flag is on", () => {
    render(
      <ReviewGatePanel
        precheck={{ outcome: "pass", verified: [] }}
        verdict={{ verdict: "pass", reason: "" }}
        patchesProposed={2}
        patchCounterEnabled={true}
      />,
    );
    expect(screen.getByText(/2 proposed/)).toBeInTheDocument();
  });

  // Placeholder state: a queued REVIEW stage opens its detail pane
  // before any events have arrived. The panel still renders to make
  // the gate's presence legible (vs. an empty card that reads as
  // "the gate did not run").
  it("renders an awaiting placeholder before any events arrive", () => {
    render(
      <ReviewGatePanel
        precheck={null}
        verdict={null}
        patchesProposed={0}
        patchCounterEnabled={false}
      />,
    );
    expect(screen.getAllByText(/awaiting/).length).toBeGreaterThanOrEqual(2);
  });
});
