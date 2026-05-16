// Surface C filter predicates and helpers. Pulled out of the page
// component so the same logic powers the count badge in the global
// nav — both surfaces apply the same "show open and newer than 14
// days by default" decay rule, so a patch hidden in the worklist
// is also out of the badge's count.

import type {
  ProposedScopePatch,
  ScopePatchKind,
} from "@/lib/rpc";

// The kind-filter buckets visible in the worklist toolbar. The wire
// enum only carries `Tighten` / `Loosen`, but the doc lists four
// buckets because the editor's mental model differentiates between
// "rule-text-only" tightens, "introduces a new predicate-enforced
// rule" patches, and "cites a paired predicate file" patches. We
// derive the extra buckets from `has_predicate` / `predicate_ref`
// rather than inventing a new wire enum.
//
// Buckets are independent — the toolbar is a multi-select. A patch
// matches the filter when *any* selected bucket accepts it (no
// buckets selected = show everything).
export type PatchKindFilter = "tighten" | "loosen" | "add" | "predicate-only";

export const PATCH_KIND_FILTERS: ReadonlyArray<{
  id: PatchKindFilter;
  label: string;
  description: string;
}> = [
  { id: "tighten", label: "Tighten", description: "Patches that narrow a rule." },
  { id: "loosen", label: "Loosen", description: "Patches that relax or remove a rule." },
  {
    id: "add",
    label: "Add",
    description: "Tightens that introduce a new predicate-enforced rule.",
  },
  {
    id: "predicate-only",
    label: "Predicate-only",
    description: "Patches that ship a paired predicate file.",
  },
];

function matchesKindFilter(
  p: ProposedScopePatch,
  bucket: PatchKindFilter,
): boolean {
  switch (bucket) {
    case "tighten":
      return p.kind === "tighten";
    case "loosen":
      return p.kind === "loosen";
    case "add":
      // "Introduces a new predicate-enforced rule" — the rulebook
      // gains a tightening that also ships the predicate to enforce
      // it. `predicate_ref` set is the load-bearing signal; the
      // `has_predicate` boolean is the parser's coarser version of
      // the same fact (kept for legacy entries that did not capture
      // the ref).
      return p.kind === "tighten" && (p.predicate_ref !== undefined && p.predicate_ref !== null);
    case "predicate-only":
      // Patches whose primary payload is a paired predicate (a
      // predicate file lands alongside or instead of prose). The
      // queue format carries `predicate_ref` only when the proposal
      // explicitly cited one.
      return p.predicate_ref !== undefined && p.predicate_ref !== null;
  }
}

// Apply the kind filter set. Empty set = no filtering (the toolbar
// renders all buckets as "off" by default; the editor opts in to
// each bucket they care about).
export function applyKindFilter(
  patches: readonly ProposedScopePatch[],
  buckets: ReadonlySet<PatchKindFilter>,
): ProposedScopePatch[] {
  if (buckets.size === 0) return [...patches];
  return patches.filter((p) =>
    [...buckets].some((b) => matchesKindFilter(p, b)),
  );
}

// Substring match against the patch's `target_path`. Case-insensitive
// because filenames are case-mixed in practice (`CLAUDE.md` vs
// `SCOPE.md`) and an editor typing `claude` should still hit both.
export function applyTargetFilter(
  patches: readonly ProposedScopePatch[],
  needle: string,
): ProposedScopePatch[] {
  const n = needle.trim().toLowerCase();
  if (n === "") return [...patches];
  return patches.filter((p) => p.target_path.toLowerCase().includes(n));
}

// Risk 5 in the doc — the worklist must default to "open AND newer
// than 14 days" so stale proposals do not turn the page into a
// graveyard. The toggle flips the cap off; entries with no
// `proposed_at` (legacy) are treated as `now` so the decay rule
// never silently hides them.
export const FOURTEEN_DAYS_MS = 14 * 24 * 60 * 60 * 1000;

export function applyAgeFilter(
  patches: readonly ProposedScopePatch[],
  showOlderThan14Days: boolean,
  now: number,
): ProposedScopePatch[] {
  if (showOlderThan14Days) return [...patches];
  const cutoff = now - FOURTEEN_DAYS_MS;
  return patches.filter((p) => {
    if (p.proposed_at === undefined || p.proposed_at === null) return true;
    return p.proposed_at >= cutoff;
  });
}

// Compose the three filters in the order the toolbar applies them.
// The order is observable through the `count` badge (which uses the
// same composition) so both surfaces stay in lock-step.
export interface FilterOpts {
  kinds: ReadonlySet<PatchKindFilter>;
  target: string;
  showOlderThan14Days: boolean;
  now: number;
}

export function applyAllFilters<T extends { patch: ProposedScopePatch }>(
  rows: readonly T[],
  opts: FilterOpts,
): T[] {
  let kept = rows.filter((r) =>
    opts.kinds.size === 0
      ? true
      : [...opts.kinds].some((b) => matchesKindFilter(r.patch, b)),
  );
  const n = opts.target.trim().toLowerCase();
  if (n !== "") {
    kept = kept.filter((r) => r.patch.target_path.toLowerCase().includes(n));
  }
  if (!opts.showOlderThan14Days) {
    const cutoff = opts.now - FOURTEEN_DAYS_MS;
    kept = kept.filter((r) => {
      const t = r.patch.proposed_at;
      if (t === undefined || t === null) return true;
      return t >= cutoff;
    });
  }
  return kept;
}

// Newest-first sort. The runtime already returns entries newest-first
// by `proposed_at`, but a client-side sort makes the order explicit
// (and survives a future change in the RPC's contract). Entries with
// `proposed_at === null` (legacy data) sink to the bottom because
// "age unknown" is observably older than any dated entry.
export function sortByNewest<T extends { patch: ProposedScopePatch }>(
  rows: readonly T[],
): T[] {
  return [...rows].sort((a, b) => {
    const ta = a.patch.proposed_at;
    const tb = b.patch.proposed_at;
    if (ta === undefined || ta === null) {
      if (tb === undefined || tb === null) return 0;
      return 1;
    }
    if (tb === undefined || tb === null) return -1;
    return tb - ta;
  });
}

// Group-by axes. `repo` is the default because workspace-attach can
// register multiple repos and the editor's natural read is "what
// does each repo's rulebook owe me?". `target` groups proposals
// editing the same file together — useful when a single rulebook
// file has accumulated patches across many jobs.
export type GroupBy = "repo" | "target";

export interface PatchListRow {
  repo_id: string;
  patch: ProposedScopePatch;
}

export interface PatchGroup {
  key: string;
  // Display label for the group header. For `groupBy=repo` the page
  // resolves the repo's name via the `useRepos` hook; for
  // `groupBy=target` the key is the file path itself.
  rows: PatchListRow[];
}

export function groupRows(
  rows: readonly PatchListRow[],
  groupBy: GroupBy,
): PatchGroup[] {
  const map = new Map<string, PatchListRow[]>();
  for (const r of rows) {
    const k = groupBy === "repo" ? r.repo_id : r.patch.target_path;
    const list = map.get(k);
    if (list) list.push(r);
    else map.set(k, [r]);
  }
  // Preserve insertion order so the row-level sort (newest-first)
  // bubbles through: the first row to land in a group decides the
  // group's position, which matches the editor's "what changed
  // recently" mental model.
  return [...map.entries()].map(([key, groupRows]) => ({ key, rows: groupRows }));
}

// Render-time helper: matches a kind to a one-letter glyph for the
// toolbar's selection chips. Kept here so the page component does
// not duplicate the mapping.
export function kindFilterGlyph(k: ScopePatchKind): string {
  return k === "tighten" ? "T" : "L";
}
