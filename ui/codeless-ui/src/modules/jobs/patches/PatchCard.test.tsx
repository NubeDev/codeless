// Card render coverage. The three actions plus the predicate-shipped
// flag and the resolved-row collapse are load-bearing for Surface B's
// "the editor decides without dropping to a terminal" journey.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PatchCard } from "./PatchCard";
import type { PatchProposal } from "./proposal";

afterEach(() => cleanup());

const baseProposal: PatchProposal = {
  id: "01TIGHTEN0000000000000000",
  review_id: "01REVIEW00000000000000000",
  stage_id: "01STAGE000000000000000000",
  kind: "tighten",
  target: "claude-md",
  target_path: "CLAUDE.md",
  evidence_stage_id: null,
  has_predicate: false,
  rationale: "helpers need a docstring naming argument types",
  body: "replace section 3 paragraph 1 with: 'every helper documents argument types'",
};

const noop = () => undefined;

describe("PatchCard", () => {
  it("renders kind, target, rationale, and the three actions for an actionable tighten", () => {
    render(
      <PatchCard
        proposal={baseProposal}
        proposedAt={Date.parse("2026-05-15T16:42:00Z")}
        resolution={null}
        onApprove={noop}
        onReject={noop}
        onApproveAfterEdit={noop}
        onEditSaved={noop}
      />,
    );
    expect(screen.getByText("tighten")).toBeInTheDocument();
    expect(screen.getByText("CLAUDE.md")).toBeInTheDocument();
    expect(screen.getByText(/docstring naming argument types/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "approve" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "reject" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "edit" })).toBeInTheDocument();
  });

  it("warns when a tighten lacks a predicate", () => {
    render(
      <PatchCard
        proposal={baseProposal}
        proposedAt={0}
        resolution={null}
        onApprove={noop}
        onReject={noop}
        onApproveAfterEdit={noop}
        onEditSaved={noop}
      />,
    );
    expect(screen.getByText("NOT SHIPPED")).toBeInTheDocument();
    expect(screen.getByText(/Tightening requires a predicate/)).toBeInTheDocument();
  });

  it("renders an evidence-stage link for loosen proposals", () => {
    render(
      <PatchCard
        proposal={{
          ...baseProposal,
          kind: "loosen",
          evidence_stage_id: "01EVIDENCE000000000000000",
          has_predicate: false,
        }}
        proposedAt={0}
        resolution={null}
        onApprove={noop}
        onReject={noop}
        onApproveAfterEdit={noop}
        onEditSaved={noop}
      />,
    );
    const link = screen.getByRole("link", { name: /01EVIDEN/ });
    expect(link.getAttribute("href")).toBe("?tab=stage:01EVIDENCE000000000000000");
  });

  it("collapses to a resolved row after approval, hiding action buttons", () => {
    render(
      <PatchCard
        proposal={baseProposal}
        proposedAt={0}
        resolution={{ kind: "approved", commit_sha: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" }}
        onApprove={noop}
        onReject={noop}
        onApproveAfterEdit={noop}
        onEditSaved={noop}
      />,
    );
    expect(screen.queryByRole("button", { name: "approve" })).toBeNull();
    expect(screen.queryByRole("button", { name: "reject" })).toBeNull();
    expect(screen.getByText(/Approved/)).toBeInTheDocument();
    expect(screen.getByText("deadbee")).toBeInTheDocument();
  });

  it("invokes onApprove when the approve button is clicked", async () => {
    const onApprove = vi.fn();
    render(
      <PatchCard
        proposal={baseProposal}
        proposedAt={0}
        resolution={null}
        onApprove={onApprove}
        onReject={noop}
        onApproveAfterEdit={noop}
        onEditSaved={noop}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "approve" }));
    expect(onApprove).toHaveBeenCalledTimes(1);
  });
});
