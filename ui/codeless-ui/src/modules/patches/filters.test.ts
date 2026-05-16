// Filter / group / sort coverage for the Surface C worklist. The
// 14-day decay rule is doc-mandated (Risk 5 in
// `DOCS/SCOPE-MUTABLE-UI.md`); the assertion below pins it so a
// future cosmetic change that flips the cutoff has to update the
// test deliberately.

import { describe, expect, it } from "vitest";

import type {
  ProposedScopePatch,
  RepoId,
  ScopePatchId,
} from "@/lib/rpc";

import {
  applyAllFilters,
  FOURTEEN_DAYS_MS,
  groupRows,
  sortByNewest,
  type PatchKindFilter,
} from "./filters";

function patch(
  over: Partial<ProposedScopePatch> & { id: string },
): ProposedScopePatch {
  return {
    kind: "tighten",
    target: "claude-md",
    target_path: "CLAUDE.md",
    rationale: "",
    body: "",
    has_predicate: false,
    evidence_stage_id: undefined,
    predicate_ref: undefined,
    fixture_ref: undefined,
    proposed_at: undefined,
    ...over,
    id: over.id as ScopePatchId,
  } as ProposedScopePatch;
}

const repo1 = "repo-1" as RepoId;
const repo2 = "repo-2" as RepoId;

describe("applyAllFilters", () => {
  it("returns every row when no filters are active", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "a" }) },
      { repo_id: repo1, patch: patch({ id: "b" }) },
    ];
    const result = applyAllFilters(rows, {
      kinds: new Set(),
      target: "",
      showOlderThan14Days: true,
      now: 0,
    });
    expect(result).toHaveLength(2);
  });

  it("filters by kind (multi-select)", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "t", kind: "tighten" }) },
      { repo_id: repo1, patch: patch({ id: "l", kind: "loosen" }) },
    ];
    const buckets = new Set<PatchKindFilter>(["loosen"]);
    const result = applyAllFilters(rows, {
      kinds: buckets,
      target: "",
      showOlderThan14Days: true,
      now: 0,
    });
    expect(result.map((r) => r.patch.id)).toEqual(["l"]);
  });

  it("treats `add` as tighten + predicate_ref present", () => {
    const rows = [
      {
        repo_id: repo1,
        patch: patch({ id: "a", kind: "tighten", predicate_ref: "p.rs" }),
      },
      { repo_id: repo1, patch: patch({ id: "b", kind: "tighten" }) },
      { repo_id: repo1, patch: patch({ id: "c", kind: "loosen", predicate_ref: "p.rs" }) },
    ];
    const buckets = new Set<PatchKindFilter>(["add"]);
    const result = applyAllFilters(rows, {
      kinds: buckets,
      target: "",
      showOlderThan14Days: true,
      now: 0,
    });
    expect(result.map((r) => r.patch.id)).toEqual(["a"]);
  });

  it("hides patches older than 14 days by default", () => {
    const now = 1_000_000_000;
    const rows = [
      { repo_id: repo1, patch: patch({ id: "fresh", proposed_at: now - 1000 }) },
      {
        repo_id: repo1,
        patch: patch({ id: "stale", proposed_at: now - FOURTEEN_DAYS_MS - 1 }),
      },
      // Undated entries are surfaced; the doc resolves "age-unknown"
      // as "treat as visible" to avoid silently hiding legacy data.
      { repo_id: repo1, patch: patch({ id: "legacy" }) },
    ];
    const result = applyAllFilters(rows, {
      kinds: new Set(),
      target: "",
      showOlderThan14Days: false,
      now,
    });
    expect(result.map((r) => r.patch.id).sort()).toEqual(["fresh", "legacy"]);
  });

  it("matches target_path case-insensitively", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "a", target_path: "DOCS/SCOPE.md" }) },
      { repo_id: repo1, patch: patch({ id: "b", target_path: "CLAUDE.md" }) },
    ];
    const result = applyAllFilters(rows, {
      kinds: new Set(),
      target: "scope",
      showOlderThan14Days: true,
      now: 0,
    });
    expect(result.map((r) => r.patch.id)).toEqual(["a"]);
  });
});

describe("sortByNewest", () => {
  it("orders dated entries newest-first, undated entries last", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "old", proposed_at: 100 }) },
      { repo_id: repo1, patch: patch({ id: "undated" }) },
      { repo_id: repo1, patch: patch({ id: "new", proposed_at: 1000 }) },
    ];
    const result = sortByNewest(rows);
    expect(result.map((r) => r.patch.id)).toEqual(["new", "old", "undated"]);
  });
});

describe("groupRows", () => {
  it("groups by repo by default", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "a" }) },
      { repo_id: repo2, patch: patch({ id: "b" }) },
      { repo_id: repo1, patch: patch({ id: "c" }) },
    ];
    const groups = groupRows(rows, "repo");
    expect(groups.map((g) => g.key)).toEqual([repo1, repo2]);
    expect(groups[0].rows.map((r) => r.patch.id)).toEqual(["a", "c"]);
  });

  it("groups by target file path", () => {
    const rows = [
      { repo_id: repo1, patch: patch({ id: "a", target_path: "DOCS/SCOPE.md" }) },
      { repo_id: repo1, patch: patch({ id: "b", target_path: "CLAUDE.md" }) },
      { repo_id: repo2, patch: patch({ id: "c", target_path: "DOCS/SCOPE.md" }) },
    ];
    const groups = groupRows(rows, "target");
    expect(groups.map((g) => g.key)).toEqual(["DOCS/SCOPE.md", "CLAUDE.md"]);
    expect(groups[0].rows.map((r) => r.patch.id)).toEqual(["a", "c"]);
  });
});
