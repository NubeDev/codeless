# Scope — session-mutable-scope

## Goal

Implement the SESSION-MUTABLE-SCOPE ramp end-to-end so that
Codeless's rulebook (SCOPE.md, CLAUDE.md, predicates) becomes a
system output rather than a static human input. After this job lands,
REVIEW stages are a real blocking gate, every WORK diff is checked
against a deterministic Layer-1 floor (diff-verify + checked-in
predicates), and REVIEW stages emit structured SCOPE-patch proposals
that earn merge by shipping their own executable check. The ramp
ends at Step 6 (human-in-the-loop approval); the doc's "deliberately
not included" steps stay not included.

The deep design is in
[`DOCS/SESSION-MUTABLE-SCOPE.md`](../../../DOCS/SESSION-MUTABLE-SCOPE.md).
This file is the per-job brief; the doc is authoritative.

## In scope

- Step 0 docs-only — JOB-MODEL.md handover spec hardening + ack-then-code
  and verify-before-handover rules in JOB-LOOP.md.
- H1/H3/H7 handover correctness fixes (per the doc's dependency note:
  ship these in the same window as Steps 0-2, not after).
- Step 1 — REVIEW as a real blocking stage type in
  `template_runner.rs`; PASS/FAIL sentinel parse; WORK-cannot-touch-
  rule-bearing-files Layer-1 file-set rule.
- Step 2 — diff-verify pre-check (Layer 1, no model invoked).
- Step 3 — predicate runner crate (xtask-shaped, host-only per R1),
  seeded with 3-5 hand-written probes.
- Step 4 — `ScopePatch` wire type in `codeless-types` (mobile-safe);
  REVIEW stages emit proposals to `DOCS/SCOPE-PROPOSED.md` in shadow
  mode; kill-criterion telemetry wired.
- Step 5 — patch-shape rules enforced at parse: tightening requires
  predicate; loosening requires positive fixture + evidence stage;
  one patch per REVIEW; mutable-set membership; evidence verification.
- Step 6 — CLI command (and/or UI affordance) for walking the
  proposed-patch queue; approved patches land as human-authored
  commits with predicate files in the same commit.

## Out of scope

- TEST stages proposing patches. The doc rules it out with reasoning;
  do not relitigate. Failing tests flag for human triage only.
- Auto-merge with a delay window. Same.
- Different-runner reviewers (claude work / codex review). Orthogonal;
  the REVIEW stage type must work runner-agnostic but selecting a
  non-default runner for REVIEW is a future job.
- Multi-tenant or per-user permissions. R5 (single-tenant trust
  boundary) is unchanged.
- Template syntax extensions beyond the new REVIEW stage type. The
  template system already supports stage types; adding two more is
  bookkeeping, not design — do not redesign templates.
- Editing the exact REVIEW prompt wording for weeks. Prompts iterate
  in days; do not block the architecture on prompt copy.
- TODO comments. Per CLAUDE.md R4, no half-finished implementations.
  Mark unfinished stages `[!]` and halt.

## Constraints

- **R1 (crate dependency direction).** The predicate runner crate
  ships under host-only crates; it must be unreachable from the mobile
  build. `std::process` / `tokio::process` stays in
  `codeless-adapters-host`. A grep for `process::Command` outside that
  crate must remain zero.
- **R2 (single transport).** Any UI affordance for Step 6 imports
  `RpcClient` only — no `@tauri-apps/api/*`, no direct `fetch` to the
  server.
- **R3 (one UI framework).** No per-shell UI files.
- **R4 (SQLite is source of truth).** Patch proposals are *file
  artifacts* in `DOCS/SCOPE-PROPOSED.md`, not a new DB table. Patch
  approval lands as a normal git commit. Do not add a new persistence
  store.
- **R5 (single-tenant trust).** Unchanged. The runtime that writes
  `SCOPE-PROPOSED.md` is the same runtime that writes handover; same
  bearer-token boundary; no new permissions.
- **Wire formats are sacred.** `DOCS/JOB-MODEL.md`,
  `DOCS/JOB-LOOP.md`, and `codeless-types/src/handover.rs` must not
  be mutable via REVIEW patches. They change via `schema_version`
  bumps and migrations. The mutable-set / wire-format-set lists in
  `codeless-runtime` config encode this; the patch parser refuses
  out-of-set targets without invoking a model.
- **`ScopePatch` is mobile-safe.** It lives in `codeless-types`, has
  no `process::Command` reachability, and no dependency on
  `codeless-runtime`.
- **Comments per CLAUDE.md.** No emojis, no task-status comments
  ("added in stage 3"), no restatements, no decorative banners. The
  comment must still make sense after the loop merges.
- **No drive-by refactors.** A stage that adds a feature does not
  bundle unrelated cleanup.

## Resolution required from "Open questions"

The scope doc lists open questions. The first stage MUST resolve
these explicitly — record the decision in
`DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md` (new file), do not silently
guess:

1. Can WORK read proposed-but-not-yet-approved patches?
2. Is RULE-DEPRECATION its own patch type, or is removal the same as
   addition? (And: prose-only loosening / deletion is fine — confirm.)
3. Does a strengthening patch re-trigger review of prior stages?
4. Where does the predicate crate live in the crate graph (exact
   crate name + Cargo.toml member entry)?
5. Predicate staleness lifecycle — deletion path that does not route
   through "WORK edits SCOPE.md."
6. Aggressiveness of prose-to-predicate promotion suggestions.

Stage 1 records the decisions; later stages cite them. A stage that
contradicts a recorded decision without amending the decisions file
is a workflow failure.

## Pointers

- Design: [`DOCS/SESSION-MUTABLE-SCOPE.md`](../../../DOCS/SESSION-MUTABLE-SCOPE.md)
- Prior doc this one replaces: `DOCS/SESSION-PEER-REVIEW-IMPROVEMENTS.md`
- Stage runner to extend: `crates/codeless-runtime/src/template_runner.rs`
- Handover contract: `crates/codeless-types/src/handover.rs`
- Workspace rules: `../CLAUDE.md` (workspace), `./CLAUDE.md` (inner repo)
