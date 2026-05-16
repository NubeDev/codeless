// Round-trip + validation coverage for the patch-edit parser. The
// runtime owns the authoritative parser, but the inbox needs a
// client-side pre-flight so an obviously-malformed edit surfaces
// without a server round-trip.

import { describe, expect, it } from "vitest";

import {
  parseProposalMarkdown,
  renderProposalMarkdown,
  type PatchProposal,
} from "./proposal";

const baseTighten: PatchProposal = {
  id: "01TIGHTEN0000000000000000",
  review_id: "01REVIEW00000000000000000",
  stage_id: "01STAGE000000000000000000",
  kind: "tighten",
  target: "claude-md",
  target_path: "CLAUDE.md",
  evidence_stage_id: null,
  has_predicate: true,
  rationale: "Helpers should carry argument types in their docstring.",
  body: "Replace section 3 paragraph 1 with: 'Every helper documents argument types.'",
};

const baseLoosen: PatchProposal = {
  id: "01LOOSEN0000000000000000A",
  review_id: "01REVIEW00000000000000001",
  stage_id: "01STAGE000000000000000001",
  kind: "loosen",
  target: "job-scope-md",
  target_path: ".codeless/jobs/x/SCOPE.md",
  evidence_stage_id: "01EVIDENCE000000000000000",
  has_predicate: false,
  rationale: "Stage 4 already covers this case.",
  body: "Delete the paragraph beginning 'No drive-by refactors'.",
};

describe("renderProposalMarkdown -> parseProposalMarkdown", () => {
  it("round-trips a tighten proposal", () => {
    const rendered = renderProposalMarkdown(baseTighten);
    const parsed = parseProposalMarkdown(rendered, baseTighten);
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.proposal.kind).toBe("tighten");
      expect(parsed.proposal.target).toBe("claude-md");
      expect(parsed.proposal.target_path).toBe("CLAUDE.md");
      expect(parsed.proposal.has_predicate).toBe(true);
      expect(parsed.proposal.evidence_stage_id).toBeNull();
      expect(parsed.proposal.rationale).toBe(baseTighten.rationale);
      expect(parsed.proposal.body).toBe(baseTighten.body);
    }
  });

  it("round-trips a loosen proposal with evidence stage id", () => {
    const rendered = renderProposalMarkdown(baseLoosen);
    const parsed = parseProposalMarkdown(rendered, baseLoosen);
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.proposal.kind).toBe("loosen");
      expect(parsed.proposal.evidence_stage_id).toBe("01EVIDENCE000000000000000");
      expect(parsed.proposal.has_predicate).toBe(false);
    }
  });
});

describe("parseProposalMarkdown — error paths", () => {
  it("rejects an empty buffer", () => {
    const r = parseProposalMarkdown("", baseTighten);
    expect(r.ok).toBe(false);
  });

  it("rejects a buffer whose first line is not `## <id>`", () => {
    const r = parseProposalMarkdown(
      "no heading here\n\n### Rationale\n\nx\n\n### Body\n\ny\n",
      baseTighten,
    );
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toMatch(/`## <patch-id>`/);
    }
  });

  it("rejects an edit that changes the patch id", () => {
    const tampered = renderProposalMarkdown(baseTighten).replace(
      baseTighten.id,
      "01DIFFERENTIDXXXXXXXXXXX",
    );
    const r = parseProposalMarkdown(tampered, baseTighten);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toMatch(/patch id changed/);
    }
  });

  it("rejects an unknown kind", () => {
    const broken = renderProposalMarkdown(baseTighten).replace(
      "kind: tighten",
      "kind: rewrite",
    );
    const r = parseProposalMarkdown(broken, baseTighten);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toMatch(/kind must be/);
    }
  });

  it("rejects a buffer missing the `### Rationale` heading", () => {
    const broken = renderProposalMarkdown(baseTighten).replace(
      "### Rationale",
      "### Why",
    );
    const r = parseProposalMarkdown(broken, baseTighten);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toMatch(/`### Rationale`/);
    }
  });

  it("rejects a buffer missing the `### Body` heading", () => {
    const broken = renderProposalMarkdown(baseTighten).replace(
      "### Body",
      "(no body heading)",
    );
    const r = parseProposalMarkdown(broken, baseTighten);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toMatch(/`### Body`/);
    }
  });
});
