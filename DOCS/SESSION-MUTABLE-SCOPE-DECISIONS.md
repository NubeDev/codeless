# SESSION-MUTABLE-SCOPE — recorded decisions

`SCOPE.md` for the `session-mutable-scope` job lists six open
questions and requires stage 1 to resolve them explicitly rather than
let later stages silently guess. This file records the resolution.
A later stage that contradicts an entry here without amending it
first is a workflow failure (per the job's `WORKFLOW.md`).

The deep design these decisions slot into is
[`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md) (forthcoming
in the workspace `DOCS/` tree; until then, the job's `SCOPE.md` is
the standing reference).

## Q1 — Can WORK read proposed-but-not-yet-approved patches?

**Decision: no.**

A WORK stage executes against the current, merged rulebook —
`SCOPE.md`, `CLAUDE.md`, and the checked-in predicates at HEAD. The
contents of `DOCS/SCOPE-PROPOSED.md` are a queue of unmerged
proposals; reading them from WORK would create two competing rule
sets (current vs proposed) and a session ambiguity that the runtime
cannot resolve. WORK stages therefore treat `SCOPE-PROPOSED.md` as
opaque.

The Step 6 approval UX (CLI and/or UI) is the only consumer of
`SCOPE-PROPOSED.md`. REVIEW stages write to it; humans walk it; WORK
stages ignore it.

**Consequence for the runtime.** The Layer-1 file-set guard adds
`DOCS/SCOPE-PROPOSED.md` to the *append-only-by-REVIEW* file set:
WORK touching it is auto-FAIL, REVIEW appending to it is allowed,
the approval CLI mutating it (to remove an approved entry) is
allowed.

## Q2 — Is RULE-DEPRECATION its own patch type, or is removal the same as addition?

**Decision: removal is the same as addition; classified as a
loosening patch. Prose-only loosening / deletion is confirmed fine.**

`ScopePatch` carries a `kind` discriminant whose values are
`Tighten` and `Loosen`. Deletion of a rule is a `Loosen`: after the
patch lands, the rule no longer constrains, which is the same effect
as a textually-narrower replacement. Same parse-time guard applies:
a `Loosen` patch must cite a positive fixture (the previously-failing
case the loosened rule now permits) plus the `evidence_stage_id` of
the stage whose diff exhibits the fixture.

Prose-only loosening — for example, removing a sentence from
`SCOPE.md` that no predicate enforces — is permitted under the same
`Loosen` rules. The "positive fixture" in that case is a stage whose
output the prior wording would have flagged and the loosened wording
permits.

**Predicate deletion is not a `ScopePatch` kind.** Predicate files
are code; deleting one is a code change. Q5 records the path.

## Q3 — Does a strengthening patch re-trigger review of prior stages?

**Decision: no.**

Patches apply *forward* from the commit that lands them. Prior
stages keep the REVIEW verdicts they earned under the snapshot of
rules in force at the time. Re-running history would (a) require
re-running entire jobs against speculative rule sets, blowing the
session-budget envelope, and (b) destabilise the audit trail, since
"PASS" would become a moving target.

The new predicate, however, must pass on **current HEAD** when the
patch lands — that is the tightening patch's positive case. The
approval flow runs the new predicate in the approving human's commit
before the patch is merged; failure blocks the merge.

**Consequence.** Sessions reading a handover from before a
strengthening patch landed do not get retroactively re-flagged. If a
new rule would have flagged an older session's diff, that is a
backlog item for human triage, not a runtime concern.

## Q4 — Where does the predicate crate live in the crate graph?

**Decision: `crates/codeless-predicates/`, package name
`codeless-predicates`.**

Added as a workspace member in the root `Cargo.toml`:

```toml
members = [
    ...,
    "crates/codeless-predicates",
]
```

The crate is host-only (R1). Its dependency closure includes
`codeless-types` (mobile-safe wire types), `tokio`, and the
process-spawning surface — which it uses **through**
`codeless-adapters-host`, not by re-implementing process spawn. A
predicate that needs to run `cargo` or `git` calls into
`codeless-adapters-host`'s exported runner; the grep "process::Command
outside `codeless-adapters-host`" must remain zero, including inside
`codeless-predicates`.

The crate is shaped like an `xtask`: a binary entry point `cargo
xtask predicates` invoked from CI and from the loop's verify step.
It is unreachable from the mobile shell's dependency closure (the
mobile shell builds `codeless-types` + `codeless-client` only). A
closure check in CI confirms this.

**Cargo.toml member entry (exact):**

```toml
[workspace]
members = [
    "crates/codeless-types",
    "crates/codeless-rpc",
    "crates/codeless-runtime",
    "crates/codeless-adapters-host",
    "crates/codeless-server",
    "crates/codeless-client",
    "crates/codeless-cli",
    "crates/codeless-tauri-desktop",
    "crates/codeless-predicates",
]
```

(Existing entries preserved; only `codeless-predicates` is added.)

## Q5 — Predicate staleness lifecycle (deletion path that does not route through "WORK edits SCOPE.md")

**Decision: predicate deletion is part of the approving human's
commit on a paired loosening patch. Never a WORK or REVIEW automated
edit.**

When a `Loosen` patch removes or weakens a rule whose enforcement
relied on a checked-in predicate, the patch's proposal cites the
predicate that becomes stale. The Step 6 approval UX surfaces both:
"approve loosening + delete predicate file" or "approve loosening +
keep predicate (it still applies to a narrower case)". The approving
human's single commit contains both the prose change and the
predicate file removal.

**Why not via REVIEW.** REVIEW stages do not write code; they
propose patches. Deleting `crates/codeless-predicates/src/foo.rs` is
a code change. Routing predicate deletion through REVIEW would
require REVIEW to land code commits — which is exactly what the
ramp's Layer-1 guards prohibit.

**Why not via WORK.** WORK cannot touch rule-bearing files, and
predicate files are rule-bearing.

**Aux case: a predicate that crashes on every diff.** That is a
bug, not a rule deprecation. Fixed via a normal human-authored
hotfix commit; same path as any other bug in code under
`crates/codeless-predicates/`.

## Q6 — Aggressiveness of prose-to-predicate promotion suggestions

**Decision: conservative. Surface candidates in the approval UI as
informational only; never auto-promote; do not block patch merge on
missing promotion.**

The approval CLI / UI flags a prose rule as a *promotion candidate*
when **both** of the following hold:

- The rule's text contains at least one phrase from a small
  allowlist of categorical words: `must not`, `never`, `always`,
  `zero`, `no <noun>`.
- The same rule has been cited as the FAIL reason in two or more
  prior REVIEW verdicts (the runtime persists REVIEW FAIL cite
  strings in the events table; the approval UX queries it).

Flagging is purely informational: the human sees "consider promoting
to a predicate" alongside the patch. The flag does not block any
patch. Writing a predicate is itself a real code change with its own
review path, and producing a flaky predicate to satisfy an
auto-promotion heuristic would be worse than leaving the rule as
prose.

**Why a conservative threshold.** A noisy prose-to-predicate
suggestion (one fired per REVIEW) trains the human to dismiss the
signal. Two-citation gating plus categorical-phrase gating biases
toward *patterns the human keeps re-explaining*, which is the
correct trigger.

## Q7 — Step 4 kill-criterion telemetry sink

**Decision: the events bus, via a new `ScopePatchProposed` event
variant on `codeless_types::Event`. No new persistence store.**

Step 4 of the ramp ships REVIEW-emitted `ScopePatch` proposals in
*shadow mode* — they accumulate in `DOCS/SCOPE-PROPOSED.md` but
nothing yet acts on them. The shadow window has an explicit
kill-criterion (the ramp doc: "if N consecutive REVIEW stages emit
zero useful proposals, the gating loop is wrong"), and answering it
requires counting proposals by stage, by reason, by acceptance
outcome. That telemetry has to flow through *some* sink.

R4 ("SQLite is source of truth; no new persistence store for patch
data") plus the existing event bus already carrying every other
stage-level signal (`ReviewRequested`, `ReviewApproved`,
`TaskCompleted`, …) makes the choice mechanical: the proposal is an
event, not a row.

**Wire shape.** A new variant added to `codeless_types::Event`:

```rust
#[serde(rename = "scope-patch-proposed")]
ScopePatchProposed {
    stage_id: StageId,
    review_id: ReviewId,
    patch_id: ScopePatchId,
    kind: ScopePatchKind, // Tighten | Loosen
    target: ScopePatchTarget, // which mutable file
    evidence_stage_id: Option<StageId>, // Loosen only
    has_predicate: bool, // Tighten only; predicate file landed in same proposal
},
```

`ScopePatchId`, `ScopePatchKind`, `ScopePatchTarget` are all
defined in the same `codeless-types` module as `ScopePatch` itself
(Step 4 land), so the event is mobile-safe by construction. The
event carries identifiers and discriminants only, not the full patch
body; consumers that need the body read `DOCS/SCOPE-PROPOSED.md` at
the commit pinned in the event's standard envelope fields.

**Aggregation.** The kill-criterion query — "how many proposals in
the last K REVIEW stages, of those how many landed, of those how
many had non-zero predicate count" — runs against the existing
events table. No new schema, no new index beyond the ones already
indexing `event_type`.

**What this rules out.** A separate `scope_patch_proposals` table
(violates R4, duplicates events). A metrics-only counter (no
per-proposal drill-down). A log line (not queryable without
ingesting log files into a second store).

## Event naming — `ReviewRequested` vs a new `ReviewGate*` family

**Decision: keep `ReviewRequested` for the existing pre-gate
human-review flow; do not rename. The Step 1 blocking REVIEW stage
type reuses the existing `Review*` event family
(`ReviewRequested` / `ReviewApproved` / `ReviewCommented` /
`ReviewStopped`); no `ReviewGate*` variants are introduced.**

Two semantically distinct things are both called "review" in this
ramp:

1. **Human review of a stage's output** — already wired:
   `ReviewRequested`, `ReviewApproved`, `ReviewCommented`,
   `ReviewStopped` in `codeless_types::Event`. The UI's Spec/Review
   pane subscribes to these.
2. **Step 1 blocking REVIEW *stage type*** — a stage in the job
   template whose `PASS` / `FAIL` sentinel decides whether the next
   stage runs. The model is the actor; the human is not in the loop
   on the hot path. The blocking gate's lifecycle hooks would
   otherwise need their own event variants (e.g.
   `ReviewGateEntered`, `ReviewGatePassed`, `ReviewGateFailed`).

Pulling this decision forward of Step 1 matters because the wire
contract for events is observable on the SSE stream — adding
`ReviewGate*` later, after subscribers have shipped, is a wire
change.

**Why reuse, not introduce a parallel family.**

- The blocking-gate stage already produces a `StageStarted` /
  `StageCompleted` pair via the existing stage-runner plumbing, plus
  a `TaskCompleted` with a `TaskStatus` that already encodes
  pass/fail. A third event family for the same transitions is
  redundant.
- The verdict comes from the `PASS:` / `FAIL:` sentinel in handover
  (per the ramp doc Step 1). The sentinel is parsed by
  `template_runner.rs`; the result is a status, not a new event
  kind. Downstream consumers that care about gate verdicts read
  `TaskStatus` off `TaskCompleted` filtered by stage-type =
  `review`.
- A `ReviewRequested` event continues to mean exactly what it means
  today: a *human* was asked to weigh in. Conflating it with the
  model-driven blocking gate would muddle subscribers (the UI's
  inbox listens for `ReviewRequested` and renders a card; a
  per-stage automated gate would spam the inbox).

**Consequence.** Step 1 lands without touching `Event`. Step 4 adds
exactly one variant (`ScopePatchProposed`, above). The schema-bump
budget for the ramp is one event variant, total.

**If a future stage needs gate-specific telemetry** — e.g. surfacing
the gate's FAIL reason without re-reading handover — that is a
follow-up with its own `schema_version` bump and migration; do not
pre-empt it here.

## Provenance

This file is authored by stage 1 of the `session-mutable-scope` job
and committed in the same commit as the stage's `JOB-MODEL.md` /
`JOB-LOOP.md` tightening. A later stage that needs to revise a
decision must amend this file in the same commit as the
revision-bearing code, and the stage's handover must call out the
amendment under `What you need to know`.
