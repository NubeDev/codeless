# SCOPE-MUTABLE-UI — recorded decisions

[`SCOPE-MUTABLE-UI.md`](../../codeless-workspace/DOCS/SCOPE-MUTABLE-UI.md)
(workspace DOCS) ends with six "open questions worth fighting about."
Stage 0 of the `scope-mutable-ui` job is to resolve those questions
explicitly so the later stages cite a single source of truth instead
of silently re-deciding. This file is that source of truth. A later
stage that contradicts an entry here without amending it first is a
workflow failure.

The deep design these decisions slot into is `SCOPE-MUTABLE-UI.md`;
the runtime ramp it sits on top of is
[`SESSION-MUTABLE-SCOPE.md`](../../codeless-workspace/DOCS/SESSION-MUTABLE-SCOPE.md)
and its prior decisions doc at
[`SESSION-MUTABLE-SCOPE-DECISIONS.md`](./SESSION-MUTABLE-SCOPE-DECISIONS.md).

The six questions are taken verbatim from the doc's "Open questions
worth fighting about" section. Five of them carry a proposed
position inside the doc; one (OQ#1) is already resolved in the
doc's Risk section. The decisions below either accept the doc's
position or override it, but never leave the question open.

## OQ#1 — A as its own job, or rolled into B?

**Decision: split. Surface A ships as its own step, before B.**

The doc's Risk #1 already records this resolution and the rest of
the document is structured around it (Step 1 = events + A; Step 3 =
RPCs + B). Recording it here so the ramp's step order is
non-negotiable from Stage 0 onwards: A must land independently of B,
and the editor's first end-to-end experience after this job is
"diagnostics in the UI, action still on the CLI." Bundling A into B
would re-open the question of whether A is worth shipping on its
own — it is, because it earns its keep as a diagnostic surface even
when patches never fire (REVIEW gates that PASS or AUTO-FAIL still
benefit from gate diagnostics).

**Consequence for the job plan.** Step 1 lands events + A on master
as a self-contained PR. No mention of patches in the UI at that
point, except that A's `Patches proposed: N` counter row is
*omitted* (not zeroed) until Step 2 lands the handover-schema fix.

## OQ#2 — Is `Patches` a tab on JobPage or a section inside `Stages`?

**Decision: tab.**

The doc's proposed position is tab-with-absence-when-empty, and the
decision accepts it. A section nested inside `Stages` would either
(a) duplicate the per-stage REVIEW gate panel (which Surface A
already covers) or (b) live at the bottom of the stages list, which
puts the editor's action surface below an unbounded scrollable list
of stage cards. Both are worse than a peer tab.

The tab is hidden when the job's `ScopePatchProposed` count is zero,
which makes the cost of the tab effectively zero on jobs that never
proposed anything. Hiding (not disabling) avoids visual noise for
the common case where REVIEW gates run but produce no proposals.

**Consequence for the runtime.** The tab visibility test is a count
of `ScopePatchProposed` events for the job. The count needs no new
RPC — the SSE event stream the JobPage already subscribes to is
sufficient. Persistence across page reload uses the same
event-replay path the existing tabs rely on.

## OQ#3 — Does the `Approve` button need a confirmation modal?

**Decision: tiered, per the doc's tentative split.**

- **Reject:** no modal. Rejection writes a rejection commit; the
  cost of an accidental reject is one re-propose by the next REVIEW
  stage that surfaces the same evidence. Friction here only slows the
  editor's workflow without protecting anything.
- **Approve from the proposal as-is:** no modal, but an undo toast
  for ~10s showing the commit sha and a one-click revert. The undo
  is a `git revert` of the approval commit (not a `git reset`), so
  the audit trail records both the approval and the undo.
- **Approve after Edit:** modal required. The modal shows a diff
  between the original proposed patch text and the edited buffer,
  plus the resolved target file. The edit path is the highest-risk
  one (typo in a predicate path, accidentally widened scope) and
  deserves the friction.

**Consequence for the RPC layer.** The approve RPC distinguishes
"approve as-is" from "approve after edit" via an optional
`edited_body` argument. Server-side validation: if `edited_body` is
present, the parser revalidates it through the same path the CLI
uses (`scope_patch_queue::parse_proposal`) before producing a commit.

## OQ#4 — Cross-window propagation when approving in JobPage updates `/patches`?

**Decision: yes. Use the existing `cross-window-events` adapter.**

The doc's proposed position is accepted in full. The
`cross-window-events.ts` shell-injected adapter at
`ui/codeless-ui/src/lib/shell/` exists for exactly this kind of
"approval in window X must invalidate worklist in window Y"
coupling, and re-using it is cheaper and more correct than inventing
a new channel.

**Consequence for the dependency table.** This decision tightens
Dependency #3 — `approve_scope_patch` / `reject_scope_patch` must
emit `ScopePatchApproved` / `ScopePatchRejected` events on the SSE
bus, so the cross-window event handler can fan out to other open
windows. The doc's #3 text already lists this requirement; recording
it here so the Stage that implements #3 cannot quietly skip the
event emit and rely on an in-process callback.

## OQ#5 — Does D's badge handle "predicate exists but has been failing for a week"?

**Decision: no. Maturity and health are separate surfaces.**

The doc's proposed position is accepted. The maturity badge answers
*"is this rule predicate-backed at all"* — a static property of the
rulebook. Predicate health answers *"is the predicate currently
green"* — a dynamic property that belongs on the CI dashboard, not
the rulebook prose. Conflating them produces a badge whose colour
shifts as CI flakes, which is worse than no badge.

The three-state rendering Surface D specifies (green / grey /
red-warning) covers a different failure mode: the cited predicate
file is *missing or unreadable*. That is a static property of the
rulebook (the annotation lies about what exists) and is the right
thing for the badge to surface.

**Consequence for Stage that ships D.** No CI integration. No
polling of test results. The badge renderer reads the
`enforced_by:` annotation, stats the cited path, and renders one of
three states. CI-driven health is explicitly out of scope.

## OQ#6 — Where do `ReviewPreCheck` / `ReviewVerdict` events appear in the timeline?

**Decision: inline in the per-stage timeline with a distinct icon;
Surface A summarises them in the gate panel rather than re-emit
them.**

The doc's proposed position is accepted. Two consequences worth
recording:

- The events appear in the SSE stream regardless of whether the
  stage detail is open. JobsDashboard's per-stage event count
  includes them — this is intended, because they are the
  highest-signal events emitted per stage, and an empty timeline
  for a REVIEW stage would imply the gate did not run.
- Surface A's gate panel is a *summary* of those events, not a
  duplicate. The panel reads the most-recent `ReviewPreCheck` and
  `ReviewVerdict` event for the stage and renders the consolidated
  view from the doc. The raw events stay visible in the timeline
  for editors who want the chronology.

**Consequence for Dependency #1.** The events must carry enough
data for both the timeline icon (verdict short label) and the
summary panel (full reason string, miss list for FAIL, verified list
for PASS). The `PreCheckOutcome` and `ReviewVerdict` shapes
proposed in the doc's #1 already capture this; recording it so the
stage that wires the events does not trim the payload to "just the
verdict enum" and then discover the panel needs more.

## Dependency table — internally consistent with the per-surface Status blocks

The doc's Dependency table:

| Surface | Backend | Frontend |
|---------|---------|----------|
| A | #1 (events); #2 if the patch counter ships | none |
| B | #2 (handover schema), #3 (RPCs + approve/reject events) | new tab + cards module |
| C | #2 + #4 (workspace-walk RPC) + proposal timestamp (see #4) | new route + module |
| D | #5 (annotation convention) | new render component with three states |

Cross-checked against each surface's Status block:

- **A.** Status says: "gate-diagnostics half unblocked once #1 lands;
  the `Patches proposed: N` counter additionally depends on #2; until
  #2 lands, ship the panel without the counter row." Table matches:
  `#1 (events); #2 if the patch counter ships`. **Consistent.**
- **B.** Status says: "blocked on PR #13 deferred-issue #6 (the
  Handover schema strips `SCOPE-PATCH-BEGIN/END` blocks)." That is
  Dependency #2. The Status block does not call out #3 as a
  pre-existing blocker because #3 is added *together with* B at
  Step 3 (it is the surface's own RPC layer, not a foreign
  dependency). Table lists both `#2` and `#3`, which is consistent
  with the Step 3 plan ("Dependency #3 (RPCs) + Surface B"). The
  ramp's step ordering — #2 lands at Step 2, then #3 + B together
  at Step 3 — reconciles the table and the Status block.
  **Consistent, with the note that #3 is co-shipped with the
  surface, not blocking it.**
- **C.** Status says: "unblocked after B. Same patch-flow plumbing;
  one extra RPC, one extra page." Table lists `#2 + #4 + proposal
  timestamp (see #4)`. Because B brings #2 first, C's incremental
  cost is `#4` plus its timestamp sub-dep. **Consistent.** The
  timestamp sub-dependency is not separately enumerated in C's
  Status block; recording here that it is bundled inside #4 (per
  the doc's "Sub-dependency" paragraph under #4).
- **D.** Status says: "mostly orthogonal. Can ship before A/B/C; the
  convention work is in `DOCS/`, the render work is a small
  component." Table lists `#5 (annotation convention)`. **Consistent.**

**No corrections to the table.** The five points where the table
appears to under-specify a Status block (B's #3, C's timestamp) are
resolved by reading the relevant Status block and Dependency #N text
together; recording the reconciliation above so later stages can
cite this section rather than re-derive it.

## What is explicitly not decided here

- **Auth, R5, R1–R4.** The doc states these are unchanged. No
  decision is needed; the existing rules apply.
- **Implementation specifics.** Wire-type field names, RPC method
  signatures, component file layout. Those land at the stage that
  ships each surface, against the design in the doc and the rules
  in `CLAUDE.md`.
- **Auto-promotion suggestions.** The doc explicitly puts this out
  of scope ("a separate job"). No decision needed; it is not on the
  ramp.
- **A dashboard widget on JobsDashboard.** Also explicitly out of
  scope ("the cross-job worklist is C; a dashboard widget would
  duplicate it").
