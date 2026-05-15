# Scope — scope-mutable-ui

## Goal

Build the user-facing surface of the SESSION-MUTABLE-SCOPE ramp.
PR #12 merged the full runtime side (REVIEW gates, diff-verify
pre-check, predicate runner, scope-patch shadow emit, parse-time
guards, CLI approval). The whole "human becomes editor of the
rulebook" role still has zero UI surface: REVIEW gate diagnostics
live only in `~/.codeless/logs/server.log`, `ScopePatchProposed`
events flow over SSE with no component rendering them, and the
`codeless patches` CLI is the only way to act on a proposal.

Ship the doc's five-step ramp:

- Step 1 — REVIEW gate panel on each REVIEW stage's detail pane
  (Surface A). Diagnostic surface; closes Journey 1.
- Step 2 — `Handover.raw_tail` (Dependency #2) so SCOPE-PATCH-
  BEGIN/END blocks survive parse-and-rewrite. Smallest unblock
  with the highest downstream EV.
- Step 3 — per-job Patches tab (Surface B). Closes Journey 2.
- Step 4 — `/patches` workspace worklist (Surface C). Closes
  Journey 3.
- Step 5 — `enforced_by:` annotation + three-state maturity
  badge (Surface D).

The full design lives in
[`DOCS/SCOPE-MUTABLE-UI.md`](../../../DOCS/SCOPE-MUTABLE-UI.md).
This file is the per-job brief; the doc is authoritative.

## In scope

- Two new event variants in `codeless-types::event`
  (`ReviewPreCheck`, `ReviewVerdict`) per Dependency #1.
- `Handover.raw_tail: Option<String>` round-trip per Dependency #2,
  backwards-compatible serde.
- Three new RPCs (`approve_scope_patch`, `reject_scope_patch`,
  `edit_scope_patch`) plus the matching `ScopePatchApproved` /
  `ScopePatchRejected` events. RPCs are idempotent and return
  `AlreadyResolved { resolution, commit_sha }` on re-call.
- One workspace-walking RPC (`list_proposed_patches(repo_id?)`)
  with a `Proposal` DTO lifted into `codeless-types` per R1.
- Add a creation timestamp to `scope_patch_queue::Proposal` at
  emit time (sub-dependency of Surface C's age filter).
- React: `ReviewGatePanel` on `StageDetail.tsx`, `Patches` tab in
  `JobTabs`, `/patches` route + module, three-state maturity
  badge on the Markdown viewer.
- `enforced_by:` annotation convention seeded on five real rules
  in `DOCS/SCOPE.md` / `DOCS/CLAUDE.md`.
- Tests: unit tests for the new event shapes, the `raw_tail`
  round-trip, the patch-resolution idempotency path, the three
  RPCs' commit-author resolution, and the badge's three-state
  render logic.

## Out of scope

- Auto-applying patches. The risk section of `SESSION-MUTABLE-
  SCOPE.md` is explicit: no auto-merge. Approve is a button that
  produces a *human-authored* commit. Do not relitigate.
- Patch *generation* in the UI. Models propose via REVIEW
  handovers; the UI has no "Add a patch manually" affordance.
- A "rerun this REVIEW gate" button. Diff-verify and PASS/FAIL
  are deterministic functions of the on-disk handover + diff.
- Patch *history* / *diff* views. Approved patches are commits;
  `git log` is the history.
- A separate audit log of who approved what. The git commit
  author IS the audit log under R5.
- Edit-in-place of `SCOPE.md` / `CLAUDE.md` outside the patch
  flow. The rule-bearing files are mutable through the patch
  flow (AI) or hand edits (human) only.
- A "Patches" widget on `JobsDashboard`. The cross-job view is
  Surface C; duplicating it on the dashboard is wrong.
- Editing predicate Rust source from the patch UI. Predicates are
  real code that wants CI feedback; the existing CodeMirror tree
  is the right surface for that.
- Auto-promotion suggestions ("rule X cited in three approved
  patches; consider writing a predicate"). Separate job, separate
  design pass.
- TODO comments in committed code. Per `CLAUDE.md` R4, no half-
  finished implementations. Mark unfinished stages `[!]` and halt.

## Constraints

- **R1 (crate dependency direction).** The new `Proposal` DTO
  lives in `codeless-types` (mobile-safe); the impl
  (`scope_patch_queue` work) stays in `codeless-runtime`. No new
  process-spawn reachable from any mobile-safe crate. Grep of
  `process::Command` outside `codeless-adapters-host` must
  remain at its current count (the new RPCs do not introduce
  spawns).
- **R2 (single transport).** Every new React module imports
  `RpcClient` only. **No** `@tauri-apps/api/*` imports in any
  of the four surfaces. The codeless-predicates probe for this
  rule (`tauri_imports.rs`) must continue to return zero matches
  against the new files.
- **R3 (one UI framework).** No per-shell files. Responsive
  design covers small-screen layouts; shell-injection adapters
  cover capability differences. Do not create
  `ReviewGatePanel.mobile.tsx` or any sibling.
- **R4 (SQLite is source of truth).** Patches remain a *file
  artifact* in `DOCS/SCOPE-PROPOSED.md` plus the git history of
  approved/rejected commits. Do NOT introduce a new SQLite table
  for patch state. The cross-window invalidation path uses the
  new resolution events on the existing bus, not a new
  persistence layer.
- **R5 (single-tenant trust).** Unchanged. Author identity is
  resolved server-side from repo-local `git config user.{name,
  email}`; the RPC takes no author args from the UI. No per-user
  permissions on patch approval.
- **Wire formats sacred.** `DOCS/JOB-MODEL.md`, `DOCS/JOB-LOOP.md`
  unchanged. `codeless-types/src/handover.rs` is *extended*
  (Dependency #2 adds `raw_tail`); this is a deliberate schema
  evolution with a backwards-compat shim, not a silent
  REVIEW-patch tightening. Bump `schema_version` and write a
  serde test that round-trips a pre-`raw_tail` handover cleanly.
- **`ScopePatch{Proposed,Approved,Rejected}` events stay
  mobile-safe.** All three live in `codeless-types`. No
  `process::Command` reachability, no dependency on
  `codeless-runtime`.
- **Comments per CLAUDE.md R2.** No emojis, no task-status
  comments ("added in stage 3"), no restatements, no decorative
  banners. Every comment must make sense after the loop merges.
- **No drive-by refactors.** A stage that adds a surface does
  not bundle unrelated UI cleanup. The Terax-conversion grind is
  a separate concern tracked elsewhere.
- **Hide-when-empty.** The `Patches` tab on JobPage is hidden
  when the job has emitted zero proposals. The /patches route
  renders an empty state with a one-paragraph orientation when
  the workspace queue is empty. Do not surface absence.

## Resolution required from "Open questions"

Stage 0 MUST resolve these into
`DOCS/SCOPE-MUTABLE-UI-DECISIONS.md`. Stage 0 is followed by a
REVIEW gate that confirms the decisions are recorded and match
the doc's Risk + Status sections before any code stage runs.

1. **OQ#1 — A as own job, or rolled into B?** Resolved by the
   doc's Risk 1: split. A ships first as a diagnostic surface;
   the CLI covers action until Step 3. Stage 0 records the
   decision verbatim so later stages do not rehash.
2. **OQ#2 — `Patches` as tab vs section?** Tab on JobPage,
   hidden when empty. Record the JobTabs `SystemTabId` change.
3. **OQ#3 — modal policy on Approve?** Resolved per doc:
   Reject = no modal; plain Approve = no modal + 10s undo toast
   with commit sha + one-click revert; Approve-after-Edit =
   modal showing diff between original proposal and edited text.
   Stage 0 records the toast lifetime and the revert RPC name
   so Stage 6 (Surface B) does not invent them.
4. **OQ#4 — cross-window event coupling?** Yes via
   `cross-window-events.ts`. Stage 0 lists the exact event
   names emitted by the resolution RPCs so Stage 9 (Surface C
   wiring) knows what to subscribe to.
5. **OQ#5 — D's badge: maturity-only or maturity+health?**
   Maturity only. Health (predicate failing on trunk) is a
   separate signal that lives on the CI dashboard, not the
   rulebook. Stage 0 records this so Stage 11 does not try to
   render a fourth state.
6. **OQ#6 — where do ReviewPreCheck/Verdict events appear in
   the timeline?** Render inline in the per-stage timeline with
   a distinct icon; the gate panel (A) *summarises* them rather
   than re-emitting. Stage 0 records the icon mapping so Stage
   4 (Surface A) and the timeline component agree on shape.

A stage that contradicts a recorded decision without amending
the decisions file is a workflow failure.

## Stop-the-job criteria

- **Stage 2 fails to ship the events without breaking the
  specta snapshot test.** If the snapshot diff is more than the
  two added variants, halt: the event enum has drifted in a way
  this stage did not author.
- **Stage 5 cannot find a working REVIEW gate to exercise.**
  The smscope-test repo at `/tmp/smscope-test` is registered;
  if it has been wiped, halt and ask the operator rather than
  re-seeding silently.
- **Stage 6 (`raw_tail`) breaks any existing handover test.**
  The backwards-compat shim is the load-bearing claim; if a
  pre-`raw_tail` handover fixture fails to round-trip, halt.
- **Stage 8 (Surface B) finds the patch event count is always
  zero in practice.** If the round-trip works in unit tests but
  no real REVIEW stage produces a proposal end-to-end, halt:
  the prompt-drift issue from PR #13 deferred-item #3 is
  unresolved and the surface has nothing to render.

## Pointers

- Design: [`DOCS/SCOPE-MUTABLE-UI.md`](../../../DOCS/SCOPE-MUTABLE-UI.md)
- Runtime this UI sits on: [`DOCS/SESSION-MUTABLE-SCOPE.md`](../../../DOCS/SESSION-MUTABLE-SCOPE.md)
- CLI this UI sits next to (does not replace): `crates/codeless-cli/src/patches.rs`
- Wire types already generated: `ui/codeless-ui/src/lib/rpc/generated/wire.ts` (search `ScopePatch`)
- Existing tab system to extend: `ui/codeless-ui/src/modules/jobs/JobTabs.tsx`
- Existing stage detail to extend: `ui/codeless-ui/src/modules/jobs/StageDetail.tsx`
- Cross-window event adapter to reuse: `ui/codeless-ui/src/lib/shell/cross-window-events.ts`
- Workspace rules: `../CLAUDE.md` (workspace), `./CLAUDE.md` (inner repo)
