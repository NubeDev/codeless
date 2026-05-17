# SCOPE-MUTABLE-UI

A design proposal for the user-facing surface of
[`SESSION-MUTABLE-SCOPE`](./SESSION-MUTABLE-SCOPE.md). Not a spec — a
thesis to argue with before any UI code lands.

Read `SESSION-MUTABLE-SCOPE.md` first. That doc describes the
runtime ramp end-to-end: REVIEW as a real blocking stage, Layer-1
diff-verify, predicate runner, shadow-mode patch proposals,
parse-time guards, CLI approval. Steps 0–6 are merged on `master` as
of PR #12; the diff-verify whitespace fix landed as PR #13.

This doc tackles the gap PR #12 left: **the loop is invisible from
the UI**. Today a user clicking through the JobsDashboard sees a
green or red stage row and nothing about *why* — no PASS/FAIL
reason, no diff-verify pre-check outcome, no list of paths the
REVIEW gate compared, and no patches the REVIEW proposed. The whole
"human becomes editor of the rulebook" role the design doc imagined
has zero surface. This doc proposes the four surfaces that fix that.

## Instructions to the reader

Before you read further: **think product, not feature list.** The
shake-up in `SESSION-MUTABLE-SCOPE.md` reshapes the human's role from
*author* to *editor*. The UI is what makes that role real — without
it, the runtime work compounds in invisible places. Specifically:

- **Reject "surface every event."** The runtime emits ~20 event
  types per stage. Most of them are noise to the editor. The UX
  question is *which signals does the editor act on*, not *what
  data can we render*.
- **Challenge the load-bearing premise.** This doc rests on one
  claim: that the editor's job is *to approve, reject, or edit
  individual patch proposals against `SCOPE.md` / `CLAUDE.md` / the
  predicate set*. If the actual job is something else — "audit
  whether the runtime got the gate right" or "tune the prompt that
  produces proposals" — the proposed surfaces are wrong. Attack
  this first.
- **Prefer one good entry point to four ok ones.** The four
  surfaces (A/B/C/D) are not equally load-bearing. A is the cheapest
  unblock; B is the per-job editor inbox; C is the cross-job
  worklist; D is a maturity badge. They compose, but the only one
  that makes the loop actually *workable* is whichever surfaces the
  patch approval flow end-to-end.
- **Name what you are willing to throw away.** The prior assumption
  was that the operator runs `codeless patches approve / reject /
  edit` from a terminal next to the dashboard. That CLI was
  explicitly the Step-6 stopping point; promoting it to a UI is an
  additive choice, not a replacement. If the UI ships, the CLI does
  not retire — operators on remote SSH boxes still need it.

If your reaction to a surface is "we could ship a smaller version of
this," you have read it right — the smaller versions are the
recommended cut. Surface A alone may be enough; B is the next-most
load-bearing; C is the optimist's bet; D is one afternoon of work.

## The thesis (one paragraph)

`SESSION-MUTABLE-SCOPE.md` ends at a CLI. That is the right stopping
point for the *engineering* ramp — adding a UI before the runtime
was provable would have been premature — but it is the wrong
stopping point for the *product*. The editor's job is to approve,
reject, or edit individual patches against the rulebook, and to
notice patterns across patches (is this rule earning its keep, is
that predicate over-firing, has this proposal been raised five times
in a row from different evidence stages and is therefore probably
real?). None of that is a `codeless patches list` job. The UI's job
is to make the editor's role *legible* — to show that REVIEW gates
ran, what they decided, what they proposed, and which of those
proposals are still in the queue waiting for a thumbs up or down.
Four surfaces, in order of how much they multiply the runtime's
value: a per-stage gate panel (A), a per-job patch inbox (B), a
cross-job patch worklist (C), and a per-rule maturity badge (D).
Build A first because it is unblocked; B is gated on a small
runtime fix; C and D compose with B once it ships.

## What is broken about the current state

After PR #12 merged the user can:

- Submit a job whose template contains REVIEW stages.
- Watch stages turn green or red in `JobsDashboard` / `JobPage`.
- Run `codeless patches list` against a hand-authored
  `DOCS/SCOPE-PROPOSED.md`.

The user *cannot*, from the UI:

- See whether a REVIEW stage's diff-verify pre-check ran, and what
  it concluded.
- See the PASS/FAIL sentinel reason the REVIEW emitted.
- See the list of paths the REVIEW gate compared (claimed-in-Done vs
  in-the-diff).
- See any `ScopePatchProposed` events emitted by REVIEW stages —
  they flow over SSE but no component renders them.
- See or interact with `DOCS/SCOPE-PROPOSED.md` (which in practice
  is empty anyway today — see "Dependencies" below).
- Approve, reject, or edit a patch proposal without dropping to a
  terminal.
- Tell at a glance which rules in `SCOPE.md` / `CLAUDE.md` are
  predicate-enforced versus prose-only.

The wire types are already generated — `ScopePatch`,
`ScopePatchKind`, `ScopePatchTarget`, and the `ScopePatchProposed`
event variant all appear in
[`ui/codeless-ui/src/lib/rpc/generated/wire.ts`](../codeless/ui/codeless-ui/src/lib/rpc/generated/wire.ts).
Nothing imports them. The runtime emits events the UI never observes.

## The six surfaces

The boundary between surfaces is the load-bearing UX decision. A
single mega-page that crams all of it together is worse than six
small ones, because the editor's task changes per surface
(diagnostic vs. approval vs. retrospective vs. **escape-hatch**
vs. **hands-off policy**). The right grouping is **per-stage /
per-job / per-workspace / per-rule / per-failure / per-job-policy**.

The original draft of this doc proposed four surfaces (A/B/C/D).
Surface E was added after a real run of the scope-mutable-ui job
got wedged at stage 1 because the deterministic pre-check
misfired and the operator had no way to advance the job without
hand-editing files on disk. The **always-finishable invariant** —
no operator workflow should require dropping to the filesystem to
unstick a job — is what surface E exists to guarantee. Without E,
A is read-only diagnostics; B/C/D require the runtime to keep
producing well-shaped output. E is what makes the *whole* loop
honest.

Surface F was added after the same job hit *four* deterministic-
gate false-positives in a row, each requiring the same operator
decision ("yeah, bypass, the gate is wrong here"). E makes the
loop *unblockable*, but it still requires the operator to be
present and decide each time. F removes the present-and-deciding
requirement for failure classes the operator has pre-canned a
response to: pick a policy at submit time and the runtime
applies that policy whenever the matching failure class occurs.
Surface F is the **hands-off operator** complement to E's
**hands-on editor**.

### Surface A — REVIEW gate panel on JobPage (per-stage)

The smallest, cheapest, most-load-bearing surface. Makes the new
gates *visible at all*.

**Where:** JobPage already has tabbed per-stage views via
[`JobTabs.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobTabs.tsx)
and per-stage detail via
[`StageDetail.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/StageDetail.tsx).
For a stage whose `is_review` flag is set, the detail pane sprouts a
**REVIEW Gate** section above the existing handover preview.

**What it shows:**

```
┌─ REVIEW GATE ─────────────────────────────────┐
│ Pre-check: PASS  (1/1 paths verified)         │
│   ✓ farewell.py — diff entry: farewell.py     │
│                                                │
│ Verdict: PASS                                  │
│   "prior Done names farewell.py and the       │
│    commit diff adds farewell.py (+2)"         │
│                                                │
│ Patches proposed: 0                            │
│ Prior handover: stage 1 → [view]              │
└────────────────────────────────────────────────┘
```

On a fail the panel shows the miss-list explicitly:

```
┌─ REVIEW GATE ─────────────────────────────────┐
│ Pre-check: FAIL  (0/1 paths verified)         │
│   ✗ WORK add foo: add bar.py                  │
│       did not appear in diff. Candidates by   │
│       basename: (none)                        │
│                                                │
│ Verdict: AUTO-FAIL (model not invoked)        │
│   "diff-verify pre-check rejected; the        │
│    handover claimed a path the commit did    │
│    not touch."                                │
└────────────────────────────────────────────────┘
```

**Why this matters:** today the same FAIL renders as a red stage
and a buried server-log line. The editor cannot tell whether the
model decided FAIL, the pre-check rejected the handover before the
model ran, or something else broke. A is the surface that turns
"red square" into "specific failure mode with the data the runtime
already produced."

**Data source:** SSE events already flow. The runtime emits a stage
start, a stage completion, and on the patches side a
`ScopePatchProposed` event. The pre-check outcome and verdict are
currently *log-only* — see "Dependencies" #1 for the small backend
change.

**Status:** the gate-diagnostics half (pre-check + verdict) is
unblocked once Dependency #1 lands. The `Patches proposed: N`
counter on the panel additionally depends on Dependency #2 — until
the Handover schema preserves `SCOPE-PATCH-BEGIN/END` blocks, the
counter renders zero on every stage regardless of what the model
emitted. Until #2 lands, ship the panel without the counter row
rather than ship a row that lies.

**Anti-pattern to avoid:** do not also render the raw assistant
transcript here. The existing handover preview already does that;
duplicating it makes the panel a wall of text. A is a *summary*
surface.

### Surface B — Per-job patch inbox (per-job)

The editor's per-job worklist. New tab on JobPage between `Stages`
and `Spec`.

**Where:** a new `Patches` system tab in
[`JobTabs.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobTabs.tsx)'s
`SystemTabId` union. The tab is hidden when the job has emitted
zero `ScopePatchProposed` events (no clutter for jobs that did not
propose anything).

**What it shows:** a vertical list of patch cards, one per
proposal, ordered by stage. Each card:

```
┌─ TIGHTEN  DOCS/CLAUDE.md ──────────────── #4 │
│                                                │
│ "all helpers should have a one-line           │
│  docstring naming argument types"             │
│                                                │
│ Evidence: stage 3 → [view diff]               │
│ Predicate: NOT SHIPPED  ⚠                     │
│   Tightening requires a predicate; this       │
│   patch will be rejected at parse time when    │
│   approve is attempted.                       │
│                                                │
│ Proposed: 2026-05-15 16:42                    │
│                                                │
│ [Approve]  [Reject]  [Edit]                   │
└────────────────────────────────────────────────┘
```

**The three actions:**

- **Approve** calls a new RPC `approve_scope_patch(patch_id)` that
  internally runs the same path as `codeless patches approve` —
  stages `DOCS/SCOPE-PROPOSED.md` + the target rulebook file + any
  referenced predicate file, produces a git commit with the
  evidence stage cited in the body. Author identity: see
  Dependency #3 — under R5 (single-tenant) there is no logged-in
  user, so the commit uses the operator's repo-local
  `git config user.{name,email}` resolved server-side at commit
  time, with a `Codeless-Approved-By: ui` trailer to distinguish
  UI approvals from CLI ones in `git log`.
- **Reject** writes a rejection commit, optionally with a reason.
- **Edit** opens an inline editor (CodeMirror, the editor module
  already exists at
  [`ui/codeless-ui/src/modules/editor/`](../codeless/ui/codeless-ui/src/modules/editor/))
  on the patch entry; on save the parser revalidates and either
  accepts or surfaces the parse error.

**Why this matters:** without it, the editor's loop is "look at the
UI to see what failed, then drop to a terminal to actually act."
That fractures the workflow and is the single biggest UX cliff
between the runtime ramp and a usable product.

**Status:** **blocked** on PR #13 deferred-issue #6 (the Handover
schema strips `SCOPE-PATCH-BEGIN/END` blocks during the
parse-and-rewrite the claude_runner does before writing the
handover to disk). Today the patch events never fire because the
markers are stripped before `scope_patch_emit::emit_from_handover`
ever sees them. The fix is small (extend `Handover` with a
`raw_tail: Option<String>` or similar that preserves the body
outside the four canonical sections); shipping B requires this
land first.

**Anti-pattern to avoid:** do not auto-apply on approve. The
editor's click is the *signal*; the actual application is the same
human-authored commit path the CLI uses. The risk section of
`SESSION-MUTABLE-SCOPE.md` is explicit on this — there is no
auto-merge mode.

### Surface C — Workspace patch worklist (per-workspace)

The editor's persistent worklist across all jobs and repos. A new
top-level page at `/patches`.

**Where:** new route in
[`app/App.tsx`](../codeless/ui/codeless-ui/src/app/App.tsx),
peer of `/jobs` and `/assistant`. New module
`ui/codeless-ui/src/modules/patches/`.

**What it shows:** the same patch cards as B, grouped by repo, with
filters by kind (Tighten / Loosen / Add / Predicate-only),
target file, and age. The page is what the editor opens when their
work that day *is* the rulebook — not a specific job.

**Why this matters:** the design doc imagines the editor spending
real time here. Patches accumulate across many jobs over weeks; the
per-job inbox of B is the right view when you are debugging a job
*right now*, but the rulebook's long-term shape needs a worklist
that outlives any single job.

**Dependency:** a new RPC `list_proposed_patches(repo_id?)` that
walks `DOCS/SCOPE-PROPOSED.md` across all repos (or one) and
returns the parsed proposal queue. The CLI parser at
[`scope_patch_queue.rs`](../codeless/crates/codeless-runtime/src/scope_patch_queue.rs)
already does the parsing; the RPC is a thin wrapper.

**Status:** unblocked *after B*. Same patch-flow plumbing; one
extra RPC, one extra page.

**Anti-pattern to avoid:** do not let C accumulate UI state the
file does not have. The mutable artifact is
`DOCS/SCOPE-PROPOSED.md` plus the git history of approved commits.
C is a view over those, not a database.

### Surface D — Rule maturity badge (per-rule)

**Status:** the docs-only half (Dependency #5) shipped along with a
CI-side validator. Eight `enforced_by:` annotations seeded across the
workspace `CLAUDE.md` (4), `DOCS/SCOPE.md` (1), and inner-repo
`codeless/CLAUDE.md` (3). The `codeless-predicates` binary grew a
`--validate-annotations [--root PATH]` subcommand that scans those
three rule files, extracts every `<!-- enforced_by: PATH -->`
annotation, and exits non-zero if any cited path is missing or not a
file — the same Layer-1 signal the in-UI red/warning pill was meant
to surface, available now via `cargo run -p codeless-predicates --
--validate-annotations` from the inner repo. Wire it into CI once a
GitHub Actions workflow exists in this repo. The UI render component
itself is deferred: the current UI has no rendered viewer for
`DOCS/SCOPE.md` / `CLAUDE.md` (the SPEC tab is a CodeMirror text
editor over per-job `.codeless/jobs/<name>/SCOPE.md`, not workspace
rule files), so the badge needs a new read-only Markdown viewer plus
the server-side `read_workspace_doc` / `check_workspace_path` RPCs
the existing wire schema does not expose. That work is a separate PR
with its own wire-schema bump.

Per `SESSION-MUTABLE-SCOPE.md`, the *artifact* of the whole ramp is
the stratification: which rules in `SCOPE.md` / `CLAUDE.md` are
predicate-enforced (Layer 1, can-fail-loudly) versus prose-only
(advisory). Surfacing that distinction at the rule level — a small
badge next to each rule heading — makes the rulebook's maturity
legible without opening any tool.

**Where:** the Markdown renderer that already exists for the
`SPEC` tab on JobPage and for the read-only documentation viewers.
A new react component that, on each `##` or `###` heading in
`DOCS/SCOPE.md` / `DOCS/CLAUDE.md`, checks the heading's
`enforced_by:` annotation (a small convention, see below) and
renders a green pill (`enforced by check_handler_instrumentation.sh`)
or a grey pill (`advisory`).

**The convention:** rule-bearing files gain a small front-matter or
heading-annotation grammar:

```markdown
## R1 — Crate dependency direction (Rust)
<!-- enforced_by: crates/codeless-predicates/src/probes/process_spawn.rs -->

The crate table in [`DOCS/SCOPE.md`...
```

**Why this matters:** today every rule in `CLAUDE.md` looks the
same to a reader. The mature-rule signal is buried in the
`codeless-predicates` crate's file list. D makes the signal
visible at the moment the human is *reading the rule* — the highest
point of leverage for "should this rule have a predicate yet?"

**Status:** mostly orthogonal. Can ship before A/B/C; the
convention work is in `DOCS/`, the render work is a small
component. The reason it is listed last is not effort — it is
because A/B/C deliver the editor's *action* loop, and D is a
*reading* affordance. Action work compounds; reading affordances
don't until there is something to act on.

**Anti-pattern to avoid:** do not auto-derive the badge by reading
predicate filenames and guessing which rule they correspond to. The
annotation is the contract; if a rule does not say
`enforced_by: …`, it is advisory. Auto-derivation introduces a
silent failure mode (predicate renamed, badge silently goes grey).

**Three states, not two.** The render component must verify the
cited path exists in the workspace and surface a third state when
it does not:

- green pill — `enforced_by: <path>` and the file exists.
- grey pill — no annotation; rule is advisory.
- red/warning pill — annotation present but the cited file is
  missing or unreadable. This catches the predicate-renamed
  silent-failure case the anti-pattern warns about.

A CI grep-validator that fails the build on broken `enforced_by:`
paths is the belt-and-braces version; the in-UI third state is the
minimum.

### Surface E — Operator escape hatch on a failed stage (per-failure)

The **always-finishable invariant.** No operator workflow should
require hand-editing a handover or dropping to the filesystem to
unstick a job. When any stage fails — REVIEW auto-fail from a
deterministic pre-check, model-emitted FAIL, runner crash,
cost-cap trip — the operator gets three buttons on that stage's
detail pane, plus a chat box that talks to the agent in the
**same Claude session** that produced the failure (if a
`session_id` was captured) or a fresh session seeded with the
failure context (if not). The buttons are deliberately
asymmetric so the user picks intent first, not action.

**Where:** the same `StageDetail.tsx` extension as Surface A,
below the gate-diagnostics panel. Hidden when the stage is
`Passed`; visible the moment the stage transitions to `Failed`.
Persists after job termination so post-mortem audits can read
the bypass history.

**The three buttons:**

```
┌─ STAGE FAILED ────────────────────────────────────────┐
│ Verdict: AUTO-FAIL (pre-check)                        │
│   "handover Done claims paths not in the diff:        │
│    codeless/scope-mutable-ui, DOCS/SCOPE-MUTABLE-UI.md"│
│                                                        │
│ [Talk to agent]  [Retry stage]  [Bypass and advance]  │
└────────────────────────────────────────────────────────┘
```

**[Talk to agent]** opens an inline chat panel that joins the
stage's Claude session (`claude --continue <session_id>` when
captured; new session seeded with the failure reason + the
prior-stage handover otherwise). The operator can ask "why did
you list the branch name?", instruct "rewrite the handover
without that bullet and re-emit", or hand the agent a
correction. The chat does *not* magically retry the stage; it
ends when the operator clicks **[Retry stage]** or **[Bypass
and advance]** on the same panel. Every message in the chat is
recorded as a `StageChatMessage` event on the existing event
bus, so the audit history is complete.

**[Retry stage]** re-runs the stage from scratch with the
chat's accumulated context prepended to the prompt. Same
session if `--continue` is in play; otherwise the chat
transcript becomes part of the seed prompt. The retry's
verdict replaces the prior failure for advancement purposes
but the original failure row stays in the database for audit.

**[Bypass and advance]** is the load-bearing button. It writes
a synthetic `Passed`-with-bypass result for the stage and
advances to the next stage. The stage row in the database
keeps its `Failed` status; a sibling field `bypassed_by:
operator` (with `bypassed_at` timestamp and optional
`bypass_reason: String`) records the override. The verdict
panel in Surface A picks up `bypassed_by:` and renders a
distinct badge: `FAILED — BYPASSED BY OPERATOR (2026-05-15
22:47)`. **The stage is never reported as Passed** by any
report or log; only the *advancement gate* honours the
bypass.

**Audit and history preservation — the rules:**

- The stage's database row stays `Failed`. Bypass does NOT
  rewrite history.
- A new `StageBypassed` event with `{stage_id, operator_reason,
  next_stage_id}` lands on the bus. It is the audit record;
  cross-window listeners (Surface C, JobsDashboard count
  badges) react to it.
- The bypass is visible in: the gate panel (Surface A's
  verdict block), the stages overview (a `BYPASSED` chip on
  the stage row), the run log (`runs/<job>/log.md` gains a
  `Session N: bypassed at stage <ord>` block).
- A bypassed stage is *never silently advanced* on any
  subsequent resume. The runtime checks `bypassed_by:` before
  re-attempting the stage and skips re-runs of an explicitly-
  bypassed stage. (Without this, a future resume would
  re-fail the stage and force a second bypass.)
- The cost of a bypassed stage is the cost spent on the
  failed attempt(s) only; bypass adds zero cost.

**Why this matters:** the alternative is what we have today —
the operator stares at a red square, hits resume, watches it
re-fail the same way, and eventually edits files on disk. That
path is hostile to the editor's role; the always-finishable
invariant is what makes the runtime production-shaped rather
than science-project-shaped.

**Status:** **the keystone surface for production readiness.**
A/B/C/D all assume the runtime keeps producing well-shaped
output. E is the recovery surface for when it does not. Ship
E in the same loop as A — both extend `StageDetail.tsx`, both
read events the runtime already produces or trivially can
produce.

**Anti-patterns to avoid:**

- **Bypass-as-success.** If the gate panel ever reads "PASSED"
  on a bypassed stage, the audit story breaks. Bypass means
  *advance*, not *pretend it passed*.
- **Chat that auto-edits the handover.** The chat is the
  operator talking to the agent. Mutations to the on-disk
  handover are explicit operator actions (a fourth button:
  `[Edit handover]` opens CodeMirror on the handover file).
  Letting the chat agent silently rewrite the handover invites
  exactly the drift the gate exists to prevent.
- **A "skip this stage" button that does NOT record the
  bypass.** Skipping without auditing is a worse failure mode
  than not advancing at all — it leaves a job whose stage
  ordering doesn't match the executed reality.
- **Auto-bypass on N retries.** Tempting; wrong. Bypass is a
  signal that the gate misfired; retrying past N failures
  without surfacing the misfire to the operator means the
  rulebook drifted away from the runtime without anyone
  noticing. The doc's `SESSION-MUTABLE-SCOPE.md` cascade-risk
  section applies: humans approve, runtime does not.

**Sub-dependency on the runtime: resume must skip Passed
stages.** Right now `TemplateRunner::run` starts at
`planned[0]` unconditionally on every invocation. That means
resume re-runs every stage from the top, including the bypassed
one. The fix is a small change: at the top of the loop, query
the stage rows for this job and skip any whose status is
`Passed` *or* whose `bypassed_by:` is set. This sub-dependency
is shared with Surface A — without it, "Why did this REVIEW
fail" gets confused by a fresh re-run replacing the failure
the operator was just looking at.

**Session continuity — same session or fresh?** Surface E's
chat affordance and the `[Retry stage]` button both depend on
whether the failing stage captured a Claude `session_id` before
it died. Three cases, each with a different behaviour:

1. **Session captured (model ran, then something failed
   after the handover landed).** Chat and retry use
   `claude --continue <session_id>`. The agent picks up the
   exact same conversation; the operator's message arrives
   as the next turn. **This is the high-context recovery
   path** and the failure mode most people imagine when
   they say "resume should pick up where it left off."
2. **Session NOT captured, but prior stage's handover
   exists (deterministic auto-fail like our diff-verify
   case — model never ran).** Chat starts a fresh Claude
   session seeded with: the stage prompt the model would
   have seen, the failure reason from the pre-check, and the
   prior-stage handover. The fresh session has the *structural*
   context but none of the *intermediate reasoning*
   (because no reasoning happened — the gate killed the
   stage before token zero).
3. **Mid-stage crash with no captured session_id** (rare:
   runner died after model started but before `session_id`
   landed in the store). Same as case 2 but with a warning
   in the chat banner: "previous session's reasoning is not
   recoverable; this is a fresh start."

The runtime distinguishes 1 from 2/3 via the stage row's
`session_id` column, which is already populated by
`StageRecorder` after each `AiMessageComplete`. No new schema
work is needed for the distinction; the chat surface just
reads the column and renders the right banner.

Your point — "handover everything it did to the next session" —
is exactly case 2's failure mode. The current handover schema
only carries the four canonical sections. A future improvement
worth tracking but **deliberately out of scope for the initial
ramp**: extend the handover to capture the model's
intermediate reasoning (tool calls, file reads, partial drafts)
so case 2 has more than the structural prompt to seed the
fresh session with. This is its own design question — the
handover wire type is mobile-safe per R1, and bloating it with
session transcripts has cost — so it lives in a separate
follow-up doc rather than this ramp. For now, case 2 is
"functional, lower-context" and the operator workaround is to
use `[Talk to agent]` in the fresh session to brief it.

### Surface F — Auto-bypass policy on a job (per-job)

The "I don't care, just code" surface. An extension of Surface E
that lets the operator pre-authorise specific kinds of failure to
**advance automatically without operator confirmation**, with a
chosen comment threaded into the next stage's prompt as
guidance. The escape hatch from E is *manual* (operator clicks
buttons or types Slack commands). F makes it *policy*: at job
submission time, the operator picks the auto-bypass behaviour
and Codeless applies it whenever the matching failure class
occurs.

**Why this matters:** the operator's role is not always "be the
careful editor of the rulebook." Sometimes the operator just
wants a long-running job to finish, and the runtime is currently
designed for the careful editor case only. Without F, every
failed gate halts the job and waits — even when the operator
already knows what they would say. F removes that waiting time
when the operator has pre-decided how they want to handle the
class of failure.

**Where:** new fields on the `Job` row (`auto_bypass_policy:
Option<AutoBypassPolicy>`) and a UI affordance on the
submit-job dialog. From Slack, set with
`@codeless resume <id> bypass auto:<preset-name>` or specified
at submit time.

**The pre-made presets:**

The job submit form (and Slack's `submit` command) takes a
single dropdown / argument with these built-in choices:

```
┌─ AUTO-BYPASS POLICY ───────────────────────────────────┐
│ ( ) None — every failure pauses for operator (default) │
│ ( ) Quick — "do what's quick and easy"                 │
│ ( ) Long-term — "only the right long-term fix"         │
│ ( ) Cheap — "pick the cheapest path forward"           │
│ ( ) Best-judgement — "use your best engineering call"  │
│ ( ) Just code — "I don't care, just keep going"        │
│ ( ) Custom — <free-text comment>                       │
└─────────────────────────────────────────────────────────┘
```

Each preset is a **canned guidance comment** the runtime
threads into the bypassed stage's *successor* (via Dependency
#2 `next_stage_comment`). Exact comment strings:

- **Quick** — *"You are auto-bypassed past the prior failed
  stage. Pick the quickest, simplest path forward; do not
  invest in long-term refactors here. If something blocks
  you again, prefer the smaller change."*
- **Long-term** — *"You are auto-bypassed past the prior
  failed stage. Choose the path that's right long-term, even
  if it costs more now; do not optimise for speed of this
  stage."*
- **Cheap** — *"You are auto-bypassed past the prior failed
  stage. Pick the path with the lowest token / wall-clock
  cost; defer anything expensive."*
- **Best-judgement** — *"You are auto-bypassed past the
  prior failed stage. Use your best engineering judgement;
  the operator trusts the call you make."*
- **Just code** — *"You are auto-bypassed past the prior
  failed stage. Do not pause to ask. Continue the job; make
  the call that lets the next stage start cleanly."*
- **Custom `<text>`** — the operator's own one-paragraph
  comment, threaded verbatim.

**How a failure is handled with each policy:**

1. Stage fails (deterministic gate auto-fail, model-emitted
   FAIL, or runner crash). Runtime checks `job.auto_bypass_policy`.
2. **None** — emit `JobFailed` event, wait for operator
   (Surface E behaviour today). No change.
3. **Any preset** — emit a `StageAutoBypassed` event with
   `{stage_id, policy, comment_used, operator: "auto"}`,
   advance to next stage with the preset's comment prefixed.
   The stage row stays `Failed` in history (same audit rule
   as manual bypass).
4. **Custom** — same as preset path, with the operator's
   text as the comment.

**Audit and history preservation — same rules as Surface E,
plus one:**

- The `StageAutoBypassed` event distinguishes auto-bypass
  from operator-clicked bypass. The verdict badge on Surface
  A reads `FAILED — AUTO-BYPASSED (policy: Quick)` rather
  than `FAILED — BYPASSED BY OPERATOR`.
- A job whose policy auto-bypassed N stages surfaces that
  count in the run log: `Session N: auto-bypassed 3 stages
  under policy "Quick"`. The editor reviewing the run sees
  the bypass count at a glance.
- A failure that auto-bypass cannot rescue (the *next* stage
  also fails — recursive bypass would mean the job is
  thrashing) **forces a halt** regardless of policy. Two
  consecutive auto-bypasses with no successful stage between
  them flips the job to `Failed` with `stop_reason:
  AutoBypassThrashing`. Operator must intervene via Surface
  E from there.

**Why a thrashing guard:** without it, a buggy gate plus an
auto-bypass policy is the failure mode that drains the cost
cap silently. The thrashing guard means auto-bypass can rescue
one bad gate per pair of stages, not a runaway loop.

**Anti-patterns to avoid:**

- **Letting auto-bypass fire on `Stop_reason: CostCap` or
  `Stop_reason: WallClockCap`.** Those are operator-set
  limits the policy must respect. A `Cheap` policy that
  auto-bypasses a cost-cap trip defeats the cap itself.
  The policy only fires on stage failures, never on
  job-level cap breaches.
- **A preset called "Skip everything."** Tempting; wrong.
  The thrashing guard catches the bad case anyway, but the
  preset list should not advertise the lazy default — every
  preset earns its keep by being *the right call for some
  failure class*.
- **Per-stage auto-bypass policy.** A job has one policy,
  full stop. Per-stage policy invites the operator to
  micromanage what was supposed to be "I don't care, just
  code." If the operator wants per-stage control, they want
  Surface E (manual bypass), not F.
- **Auto-bypass on a REVIEW stage that proposed a SCOPE
  patch.** The patch belongs in the editor's queue (Surface
  B). Auto-bypassing past it skips the patch proposal
  signal entirely. The runtime captures the patch first
  (it survives in `DOCS/SCOPE-PROPOSED.md` via Dependency
  #2's `raw_tail`), then applies the policy — the patch is
  never lost, but the gate that surfaced it is bypassed.
- **Slack's `bypass` shortcut implying a default policy.**
  A bare `resume <id> bypass` is a one-shot manual bypass,
  not a policy change. Setting / changing the job's policy
  is its own command (`@codeless policy <id> <preset>`).
  Conflating the two means the operator who wanted one
  bypass accidentally sets a permanent policy.

**Status:** **builds on E.** F is unblocked once E ships
(Dependency #6a + #6b) plus a small policy column on the Job
row. Adds maybe ~100 lines to the runtime (policy field +
auto-bypass branch in the stage-failed handler + thrashing
guard) plus the submit-form preset picker (~30 lines) and the
Slack command (~10 lines).

**Sub-dependency: `next_stage_comment` already exists in the
plan.** The Slack integration (Dependency #2 from
`SCOPE-SLACK-INTEGRATION.md`) ships `next_stage_comment` on
`ResumeJobArgs`. F reuses the same field — the runtime path
that injects the comment into the next stage's prompt is
shared. F does not add a parallel mechanism; it adds a
*source* of the comment (the policy preset) alongside the
existing operator-typed source.

## The user journeys

Four journeys the surfaces have to support, in priority order.

### Journey 1 — "Why did this REVIEW stage fail?"

The editor is debugging a red job. Today they look at
`JobsDashboard`, see one red stage, click into `JobPage`, see the
red stage's detail pane, and read … a handover that says the agent
emitted a `PASS:` somewhere but the parser disagreed. The runtime's
*specific* diagnosis lives in `~/.codeless/logs/server.log`. That
is the cliff.

With A: the stage detail's REVIEW Gate panel shows the verdict
(unparseable / fail / pass-with-reason), and on a pre-check fail,
the exact miss list with candidates. The editor's diagnostic loop
collapses from "tail the server log, grep for the job id" to "click
the stage, read the panel."

### Journey 2 — "Approve or reject the patches this job produced"

The editor's per-job review pass. After a job that ran a few REVIEW
stages with patch proposals, the editor opens JobPage, clicks the
new `Patches` tab (only present when proposals exist), and walks
the list. Each card carries everything needed to decide: kind, the
target file, the rationale, the evidence stage (with a link to its
diff), and the predicate-shipped flag.

The action is one click — `[Approve]` writes the commit, `[Reject]`
writes a rejection commit, `[Edit]` opens the patch entry for hand
editing.

With B unblocked, this journey replaces "open a terminal, run
`codeless patches list`, copy the patch id, run `codeless patches
approve <id>`" with three clicks.

### Journey 3 — "Walk the open patch queue across the project"

The editor's weekly worklist. Not "what did this job propose" but
"what is the rulebook waiting on me to decide." Surface C is the
page they open Monday morning, with a count badge in the global
nav.

Filters and grouping matter here in a way they do not on B (where
the natural grouping is *this job*). The grouping rules:

- By repo (one workspace can attach multiple).
- By target file (one rulebook file at a time is easier to reason
  about).
- By age (old proposals decay; a six-week-old patch is probably
  stale, surface them at the bottom).

Sort default: newest first, grouped by target file.

### Journey 4 — "A stage failed and I need this job to finish anyway"

The escape-hatch journey. The deterministic pre-check misfired
on a path-shape false positive, or the model emitted a FAIL the
operator disagrees with, or a runner crash left the stage in a
state the operator wants to advance past. The job has invested
real tokens in earlier stages; throwing the whole job away is
not the right answer.

With E: the operator clicks into the failed stage, sees the
verdict + reason in Surface A's gate panel, and chooses one of
three actions on the panel below:

- **[Talk to agent]** — opens chat with the failing stage's
  Claude session. The operator asks "why did your `Done`
  bullet name the branch?" and the agent explains. The
  operator instructs "regenerate your handover without
  mentioning the branch by name." The agent does, and the
  operator clicks **[Retry stage]**, which re-runs the stage
  with the corrected handover. Most failures resolve here.
- **[Retry stage]** alone — re-runs the stage with the chat
  context (if any) prepended. The simplest path when the
  failure was transient (network blip, model timeout).
- **[Bypass and advance]** — the editor's escape valve. Marks
  the stage `Failed` in history, records a `StageBypassed`
  event with a reason string, and moves on to the next stage.
  Used when the gate misfired and retry won't help (the
  operator has decided the gate was wrong; logging the
  override is the audit trail).

Without E: the operator has none of these. The pattern today
is "hit resume, watch it re-fail the same way, eventually edit
the handover file on disk." That is hostile to the editor's
role and breaks the always-finishable invariant.

### Journey 5 — "I don't care, just code"

The hands-off operator journey. The operator submits a job
they want to *finish*, not *audit*. They pick a policy at
submit time (e.g. `Quick` or `Just code`), step away, and
return to either a completed job or a `Failed` row with
`stop_reason: AutoBypassThrashing` and a clear failure
history.

With F: stages that auto-fail under a deterministic gate (a
false-positive pre-check, an over-strict review verdict) are
silently advanced with a canned guidance comment threaded
into the next stage's prompt. The audit trail keeps every
auto-bypass row as `Failed`; the run log summarises the
count. Two consecutive auto-bypasses with no success between
them halts the job — the operator can either change the
policy (`@codeless policy <id> Long-term`) and resume, or
take manual control via Surface E.

Without F: the operator who *did* want to walk away has to
keep an eye on Slack notifications and manually answer each
failure with the same guidance they would have pre-canned.
For long jobs run overnight or under cost caps, F is what
turns "babysit the job" into "submit it and check back."

This journey is mutually exclusive with Journey 1 ("Why did
this REVIEW fail") in a sense — Journey 1 wants the
diagnostic, Journey 5 wants the result. The operator picks
their stance per job, not globally; the same operator can
submit one job with `auto_bypass: None` (audit mode) and the
next with `auto_bypass: Quick` (hands-off mode).

## Dependencies — what has to land before each surface

### Dependency table

| Surface | Backend | Frontend |
|---------|---------|----------|
| A (gate panel) | #1 (events); #2 if the patch counter ships | none |
| B (per-job inbox) | #2 (handover schema), #3 (RPCs + approve/reject events) | new tab + cards module |
| C (worklist) | #2 + #4 (workspace-walk RPC) + proposal timestamp (see #4) | new route + module |
| D (rule badge) | #5 (annotation convention) | new render component with three states |
| E (escape hatch) | #6 (bypass + chat-on-stage + resume-skips-Passed) | extends `StageDetail.tsx` alongside A |
| F (auto-bypass policy) | #7 (policy column + thrashing guard) and Slack-doc #2 (`next_stage_comment`) | preset picker on submit form (~30 lines); policy badge on JobPage header |

### Dependency #1 — REVIEW gate outcome flows over events

Currently
[`template_runner.rs`](../codeless/crates/codeless-runtime/src/template_runner.rs)
logs the pre-check outcome (`diff-verify pre-check: every claimed
path resolved to a diff entry count=1`) and the verdict (`review
gate passed reason=...`) via `tracing::info!` / `tracing::warn!`.
For A the UI needs them as events.

Two new event variants, in
[`codeless-types/src/event.rs`](../codeless/crates/codeless-types/src/event.rs):

```rust
Event::ReviewPreCheck {
    stage_id: StageId,
    outcome: PreCheckOutcome, // {Pass{verified}, Fail{missing}, Skipped, NothingToVerify}
},
Event::ReviewVerdict {
    stage_id: StageId,
    verdict: ReviewVerdict,   // {Pass{reason}, Fail{reason}, AutoFail{reason}}
},
```

Cost: small. Same `codeless-types` ergonomic shape as
`ScopePatchProposed`. Mobile-safe.

### Dependency #2 — Handover schema preserves SCOPE-PATCH blocks

Open as deferred-issue #6 in PR #13. Today the claude_runner
extracts the handover fenced block, parses it through
`Handover::from_markdown`, and writes the on-disk file via
`Handover::to_markdown`. Only the four canonical sections
round-trip; any `SCOPE-PATCH-BEGIN/END` block in the body is
silently dropped. `scope_patch_emit::emit_from_handover` then reads
the on-disk file and finds nothing.

Two paths:

- (a) Tell the agent to wrap the patch *inside* a `Done` bullet
  using a backtick-fence the parser ignores. Convention-only fix
  but fragile — bullet parsers munch lines, and the marker uses
  exact-line equality (`trimmed == BEGIN_MARKER`).
- (b) Extend `Handover` with a `raw_tail: Option<String>` field
  that captures everything after `## Open questions` verbatim and
  round-trips it. The marker parser reads from the on-disk file
  unchanged.

Path (b) is the right call. Requires a small wire-version bump or
backwards-compat shim in the serde for old handovers.

### Dependency #3 — `approve_scope_patch` / `reject_scope_patch` RPCs

The CLI at
[`codeless-cli/src/patches.rs`](../codeless/crates/codeless-cli/src/patches.rs)
already does the work. Wrap each subcommand in an RPC. The RPC
returns the resulting commit sha so the UI can link to it.

Three non-obvious requirements:

- **Idempotent / race-tolerant.** Two windows can show the same
  patch. If window A approves, window B's later call must return a
  structured `AlreadyResolved { resolution: Approved | Rejected,
  commit_sha }` result rather than an error, so window B can
  refresh its view without surfacing a red error toast.
- **Author identity.** Resolved server-side from the repo-local
  `git config user.{name,email}` (see Surface B). The RPC takes no
  author args from the UI — R5 means the UI has no identity to
  send.
- **Emits `ScopePatchApproved` / `ScopePatchRejected` events.**
  Cross-window invalidation (Open Question #4) needs an event to
  hang off; the existing `ScopePatchProposed` covers proposal but
  not resolution. Same shape, mobile-safe.

### Dependency #4 — `list_proposed_patches(repo_id?)` RPC

Wrap `scope_patch_queue::load_queue` over one or all repos. Returns
the same `Proposal` shape the CLI uses, lifted to `codeless-types`
so the wire DTO crosses the mobile-safe boundary cleanly
(implementation stays in `codeless-runtime`, which is host-only per
R1).

**Sub-dependency:** the `Proposal` shape needs a creation timestamp
for Surface C's age filter and decay sort. If `scope_patch_queue`
does not currently capture one, add it at proposal-emit time. Without
it, C's "older than 14 days falls below the fold" default is not
implementable and should be dropped from C until the field exists.

### Dependency #5 — `enforced_by:` annotation convention

Smallest dependency. A docs-only convention; the badge component
reads the annotation directly. No schema change.

### Dependency #6 — Bypass + stage chat + resume-skips-Passed

Three small backend changes that together unlock Surface E.

**6a. `resume_job` grows a `bypass: Option<BypassRequest>`
arg.** When set, the resume marks the most recently failed
stage as bypassed before requeuing. The `BypassRequest` shape:

```rust
pub struct BypassRequest {
    pub stage_id: StageId,
    pub reason: Option<String>,
}
```

Persists as a sibling column on the stages table:
`bypassed_at: Option<i64>`, `bypassed_reason: Option<String>`.
The stage's `status` column stays `Failed`; advancement logic
checks `bypassed_at.is_some()` to decide whether to advance.

Emits a `StageBypassed` event with `{stage_id, operator_reason,
next_stage_index}` for cross-window listeners.

**6b. `TemplateRunner::run` skips Passed-or-bypassed stages.**
At the top of the per-stage loop, query the stage rows for
this job from the store. If a row exists for this stage
ordinal AND its status is `Passed` OR its `bypassed_at` is
set, skip the inner body (still emit `StageStarted` /
`StageCompleted` with a `skipped: true` flag so the UI shows
the timeline correctly). This makes resume an *advance*, not
a restart-from-zero. Sub-dependency of E but also fixes the
pre-existing "resume restarts at stage 0" bug filed in PR #14.

**6c. Stage-scoped chat RPC.** A new `stage_chat(stage_id,
message)` RPC that:

- Looks up the stage's captured `session_id` (if any).
- If captured, invokes `claude --continue <session_id>` with
  the operator's message; the agent's reply emits a
  `StageChatMessage` event.
- If not captured (auto-fail before model invocation),
  starts a fresh Claude session seeded with the stage prompt
  + failure reason + prior handover.
- Records the full exchange as `StageChatMessage` events on
  the bus so the audit trail is complete.

The RPC does *not* mutate the on-disk handover or re-run the
stage. Those are explicit operator actions on the gate panel
(`[Retry stage]`, `[Bypass and advance]`).

**Mobile-safety:** `BypassRequest`, the chat-message event,
and the stage-skipped flag all live in `codeless-types`. The
impl (`stage_chat`, the bypass write path, the resume-skip
loop) lives in `codeless-runtime` per R1.

### Dependency #7 — Auto-bypass policy + thrashing guard

Three pieces that together unlock Surface F.

**7a. `Job.auto_bypass_policy` column.** A new `Option<...>`
field on the `Job` row:

```rust
pub enum AutoBypassPolicy {
    Quick,         // canned: "quick and easy"
    LongTerm,      // canned: "right long-term"
    Cheap,         // canned: "cheapest path"
    BestJudgement, // canned: "use your judgement"
    JustCode,      // canned: "don't pause, just go"
    Custom(String),// operator's free-text comment
}

pub struct Job {
    // ... existing fields
    pub auto_bypass_policy: Option<AutoBypassPolicy>,
}
```

Set at submit time via `SubmitJobArgs.auto_bypass_policy`.
Set / changed mid-job via a new
`set_job_policy(job_id, policy)` RPC, which only succeeds
when the job is not currently `Running` (operator pauses,
sets, resumes).

The canned comment strings live in
`codeless-runtime::auto_bypass_policy` as `const &str` values
so they're version-controlled, reviewable, and don't drift
without a code change.

**7b. Stage-failed handler reads policy and auto-bypasses.**
The runtime branch that today emits `JobFailed` and halts on
stage failure becomes a `match` on the policy:

```rust
match job.auto_bypass_policy {
    None => /* existing: halt, emit JobFailed */,
    Some(policy) if thrashing_detected(job, stage) => {
        /* halt with stop_reason: AutoBypassThrashing */
    }
    Some(policy) => {
        /* mark stage `Failed` with `bypassed_at: now(),
         *  bypassed_reason: policy_comment(policy)`,
         *  emit StageAutoBypassed event, advance */
    }
}
```

The policy's canned comment becomes the next stage's
`next_stage_comment` (reusing the Slack-doc Dependency #2
plumbing). No parallel comment-injection mechanism.

**7c. Thrashing guard.** A simple two-strikes rule: track the
last two stage outcomes per job. If both are
`Failed`-then-auto-bypassed with no `Passed` between them, the
job halts with `stop_reason: AutoBypassThrashing` and the
operator must intervene via Surface E (manual bypass with
custom guidance, or stop the job).

The two-strikes window is deliberately short. A longer window
(e.g. "three in a row") would let buggy gates burn more cost
before the guard fires; a shorter one (one) defeats the
auto-bypass purpose. Two is the smallest number that keeps
the auto-bypass *useful* (rescues a single bad gate) without
letting it *loop* (rescues many in a row).

Cost-cap and wall-clock-cap breaches are NOT subject to the
policy — those are operator-set limits the policy must
respect, per the anti-pattern list in Surface F. The
auto-bypass branch never fires when the failure reason is
`Stop_reason: CostCap` or `Stop_reason: WallClockCap`.

**Mobile-safety:** `AutoBypassPolicy` and `StageAutoBypassed`
event live in `codeless-types`. Thrashing-detection state is
per-job, ephemeral, in `codeless-runtime`.

**Audit:** every auto-bypass emits a `StageAutoBypassed`
event with `{stage_id, policy_name, comment_used,
applied_at}`. The stage row stays `Failed`. The run log
gains a per-session summary line: `auto-bypassed N stages
under policy "<name>"`.

## What this ramp deliberately does not include

- **Automatic patch application.** The risk section of
  `SESSION-MUTABLE-SCOPE.md` is explicit: there is no auto-merge
  mode. The UI honours this — approve is a button that writes a
  *human-authored* commit, with the human's git identity.
- **Patch *generation* in the UI.** The model proposes patches via
  REVIEW stage handovers; the UI does not have an "Add a patch
  manually" affordance. If the editor wants to add a rule directly,
  they edit `SCOPE.md` and commit normally.
- **A "rerun this REVIEW gate" button.** Diff-verify and PASS/FAIL
  are deterministic functions of the on-disk handover + diff. A
  rerun button would only make sense after editing the handover,
  which is not a workflow we want to encourage.
- **Patch *history* / *diff* views.** Approved patches are commits;
  `git log` shows them already. Reinventing that view inside the
  app is not what the editor needs.
- **A separate audit log of who approved what.** The git commit
  author is the audit log. R5 (single-tenant) means one editor
  anyway.
- **Edit-in-place of `SCOPE.md` / `CLAUDE.md` from the UI.** The
  rule-bearing files are mutable only through the patch flow (for
  the AI) or hand edits (for the human). The UI does not turn
  these files into an editor surface — that would invite
  drive-by edits that the runtime's Layer-1 gate is meant to
  prevent.

## Risk and the failure modes

**Risk 1 — A ships, B / C do not.** Editor sees gate diagnostics
but still cannot act on patches without the CLI. The loop is
*observable but not actionable*. Mitigation: accept this as the
intentional Step-1 stopping point. A is a diagnostic surface that
earns its keep without B (it explains red stages); the CLI remains
load-bearing for *action* until Step 3. This resolves Open Question
#1 in favour of split-and-ship-A-first.

**Risk 2 — D ships before A.** Reading affordance with no action
affordance. The editor sees which rules are predicate-enforced but
cannot influence the next batch of patches. Mitigation: order is
A, B, (C ∥ D), not D first.

**Risk 3 — The Handover schema fix (dependency #2) is harder than
it looks.** The handover wire type is on the *mobile-safe* side of
the crate graph. Extending it bumps the schema version and may
require a migration. Mitigation: prototype the `raw_tail` shape in
a throwaway branch *before* committing to B's surface.

**Risk 4 — Patch volume in B is low, so the tab is mostly empty.**
B's value is concentrated in the few stages that actually emit
patches. If most jobs do not, the tab feels under-used. Counter:
the `Patches` tab is hidden when zero patches exist, so it does not
add cognitive load when not relevant.

**Risk 5 — The cross-job worklist (C) becomes a graveyard.** Old
proposals accumulate; nothing forces decision. Mitigation: the
filter UI defaults to "open AND newer than 14 days." Older patches
need an explicit click to surface, on the theory that stale
proposals deserve a manual triage step.

## Pointers

- The runtime this UI sits on: [`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md)
- The CLI this UI sits next to (does not replace):
  [`codeless-cli/src/patches.rs`](../codeless/crates/codeless-cli/src/patches.rs)
- The wire types already generated:
  [`ui/codeless-ui/src/lib/rpc/generated/wire.ts`](../codeless/ui/codeless-ui/src/lib/rpc/generated/wire.ts)
  (search `ScopePatch` / `ScopePatchProposed`)
- The deferred runtime issues this doc folds in as dependencies:
  PR #13 body, items #3 (REVIEW prompt drift), #5 (auto-fail stub
  handover), #6 (Handover schema preserves SCOPE-PATCH blocks).
- The job UI surfaces this proposal extends:
  [`JOB-UI.md`](./JOB-UI.md), [`JOBS-UX.md`](./JOBS-UX.md)

## Open questions worth fighting about

1. **A as its own job, or rolled into B?** Resolved (see Risk 1):
   split. A ships first as a diagnostic surface; the CLI covers
   action until Step 3.
2. **Is `Patches` a tab on JobPage or a section in `Stages`?** A
   tab is the right call when the proposal count is unbounded; a
   section is right when it is small. Real jobs probably emit 0–3
   patches. Probably: tab, because the absence-when-empty rule
   makes the cost zero on patch-less jobs.
3. **Should the `Approve` button require a confirmation modal?**
   The action writes a commit to the rulebook the runtime reads —
   reversible, but every future job runs against the new gate
   behaviour until reverted. Tentative split:
   - **Reject:** no modal. Pure friction; rejection is cheap to
     undo (re-propose).
   - **Approve from the proposal as-is:** no modal, but render an
     undo toast for ~10s with the commit sha and a one-click
     revert.
   - **Approve after Edit:** modal required, showing the diff
     between the original proposal and the edited text. The edit
     buffer is the highest-risk path (typo in a predicate path,
     accidentally widened scope) and deserves the friction.
4. **Cross-window: does approving a patch in JobPage update the
   `/patches` worklist (C)?** Yes — the
   `cross-window-events.ts` adapter already exists for exactly
   this kind of "approval in window X must invalidate worklist in
   window Y" coupling. Use it.
5. **Does D's badge need to handle the case "predicate exists but
   has been failing for a week"?** Argument for: a green badge
   that is silently broken is worse than no badge. Argument
   against: predicate status is a separate signal; mixing it into
   the maturity badge muddies what the badge says. Probably:
   separate. Maturity is "is there a predicate at all"; health is
   a different surface (CI dashboard, not the rulebook).
6. **Where do `ReviewPreCheck` / `ReviewVerdict` events appear in
   the timeline / event log?** They are the highest-signal events
   per stage. Probably: render them inline in the existing
   per-stage timeline with a distinct icon, and let A *summarise*
   them in the gate panel rather than re-emit them.

## What ships, in order

A ramp, not a tier list. Each step independently useful, each makes
the next cheap.

### Step 1 — Dependency #1 (events) + Surface A (Layer 2) + Surface E foundation

Add `ReviewPreCheck` and `ReviewVerdict` events to
`codeless-types`; emit them from `template_runner.rs` wherever the
existing logs fire. Wire the events into a new `ReviewGatePanel`
React component on `StageDetail.tsx`. **Co-ship Dependency #6a
and #6b**: the `bypass` arg on `resume_job` plus the resume-
skips-Passed-stages runtime fix. UI side ships the **[Bypass and
advance]** button on `StageDetail.tsx` for any `Failed` stage.

Dependency #6c (stage chat RPC) is deferred to Step 6 — the
chat surface is more work than a button click and the
`[Bypass and advance]` button alone restores the always-
finishable invariant.

Stopping here: the editor has full gate-failure diagnostics
AND a one-click escape hatch for any failed stage. The CLI is
still the only way to act on patches, but the *observe* half
of the loop is closed AND the *recovery* half is closed for
the no-context-needed case.

### Step 2 — Dependency #2 (Handover schema fix)

Extend `Handover` with `raw_tail: Option<String>`; thread it
through `from_markdown` / `to_markdown`; ensure
`emit_from_handover` reads from the file unchanged. Backwards-
compatible serde shim for prior handovers (no `raw_tail` = empty
string).

This is the smallest unblock with the highest downstream EV — every
later step depends on it, and shipping it alone closes deferred
issue #6 from PR #13.

### Step 3 — Dependency #3 (RPCs) + Surface B (Layer 2)

Add `approve_scope_patch` / `reject_scope_patch` / `edit_scope_patch`
RPCs around the existing CLI logic. Surface a `Patches` tab on
JobPage that lists this job's proposed patches as approve/reject
cards. Hide the tab when proposal count is zero.

Stopping here: the per-job editor loop is fully UI-driven. The
cross-job worklist still requires the CLI.

### Step 4 — Dependency #4 (workspace-walk RPC) + Surface C (Layer 2)

Add `list_proposed_patches(repo_id?)`. New `/patches` route + module
mirroring the per-job cards, with filters by kind / target / age.
Cross-window events propagate approvals between JobPage and
`/patches`.

Stopping here: the editor's full action loop, both per-job and
cross-job, lives in the UI. The CLI persists for remote / scripted
use.

### Step 5 — Dependency #5 (convention) + Surface D (Layer 2)

`enforced_by:` annotation convention in `DOCS/SCOPE.md` and
`DOCS/CLAUDE.md`. Small render component on the existing Markdown
viewers reads the annotation and pins a green or grey badge. Seed
five rules with annotations as a starting set.

### Step 6 — Dependency #6c (stage chat) + Surface E completion

Add the `stage_chat(stage_id, message)` RPC and wire it to a
chat panel on `StageDetail.tsx` next to the existing
**[Bypass and advance]** button (which shipped in Step 1).
Same-session continuation when the failing stage captured a
`session_id`; fresh-session seeded with failure context
otherwise. Add **[Retry stage]** which re-runs the stage with
the accumulated chat context as a prompt prefix.

Stopping here: Surface E is complete — failing stages are
fully recoverable from the UI via talk-then-retry or
talk-then-bypass paths, in addition to the no-context-needed
direct bypass that shipped in Step 1.

### Step 7 — Dependency #7 + Surface F (auto-bypass policy)

Add `Job.auto_bypass_policy` column, the stage-failed branch
that auto-bypasses with the policy's canned comment, the
thrashing guard, and the `StageAutoBypassed` event. UI:
preset picker on the submit-job dialog; a small policy badge
on the JobPage header so the operator sees "policy: Quick"
at a glance. Slack: `@codeless policy <id> <preset>` to set
or change.

Stopping here: the "I don't care, just code" workflow is
operational. Operators can submit a job under any preset
(or a custom comment) and walk away; the runtime advances
through false-failing gates with pre-decided guidance,
halts only on real thrashing or cap breaches.

The ramp ends here. Auto-promotion suggestions ("rule X has been
cited in three approved patches; consider writing a predicate")
are a *separate* job — they are an editor-action surface, not a
reading surface, and they want their own design pass.

### Stopping points

- Stop at Step 1: the editor sees the gate AND the always-finishable
  invariant holds. Diagnostic loop closed; one-click bypass available
  for any failed stage. The most important early stopping point —
  without it the operator's loop is broken in production.
- Stop at Step 3: per-job patch loop closed; CLI still load-bearing
  for cross-job work.
- Stop at Step 4: full editor loop in the UI.
- Stop at Step 5: the rulebook's maturity is visible at the moment of
  reading.
- Stop at Step 6: failing stages have full talk-then-retry recovery
  in addition to the direct bypass that shipped in Step 1.
- Reach Step 7: the hands-off "just code" operator mode is live;
  jobs run unattended under a chosen policy with the thrashing
  guard as the safety net.

Each stopping point is a real win. **Step 1 is the production-
readiness floor** (always-finishable). Step 3 is where the editor's
*action* loop becomes UI-native; Step 4 is where it becomes a
worklist; Step 5 is the polish that makes the long-term artifact
(the stratified rulebook) legible; Step 6 is the high-context
recovery path that turns chat into a real recovery surface; **Step
7 is the hands-off operator mode** that lets an operator submit a
job and walk away.

### What this ramp deliberately does not include

- **A "Patches" dashboard widget on JobsDashboard.** The
  cross-job worklist is C; a dashboard widget would duplicate it
  with a smaller window. If you find yourself wanting one, the
  signal is "C should have a count badge in the global nav," not
  "JobsDashboard needs a fifth widget."
- **Editing predicate Rust source from the UI.** Predicates are
  real code that wants CI feedback. The editor's UI handles the
  prose and the approval; predicate authoring stays in the
  editor's editor (CodeMirror file tree is fine; turning the
  patch UI into a Rust IDE is out of scope).
- **A "request a re-review" button on a job whose REVIEW already
  PASSED.** That would re-enter the gate against the same diff
  and produce the same verdict (the check is deterministic). If a
  rule changed since the gate ran, the right move is a new job
  against the new rule — not a rerun of an old gate.

## What lands where in the codebase

The "reuses what exists" claim has to be auditable. Touchpoints:

- [`crates/codeless-types/src/event.rs`](../codeless/crates/codeless-types/src/event.rs)
  — two new event variants (`ReviewPreCheck`, `ReviewVerdict`),
  same shape conventions as `ScopePatchProposed`.
- [`crates/codeless-types/src/handover.rs`](../codeless/crates/codeless-types/src/handover.rs)
  — adds `raw_tail: Option<String>` and threads it through the
  markdown round-trip.
- [`crates/codeless-runtime/src/template_runner.rs`](../codeless/crates/codeless-runtime/src/template_runner.rs)
  — emits the new events where the existing tracing calls fire.
- [`crates/codeless-runtime/src/rpc/`](../codeless/crates/codeless-runtime/src/rpc/)
  — new methods `approve_scope_patch`, `reject_scope_patch`,
  `edit_scope_patch`, `list_proposed_patches`. Each wraps existing
  logic in `scope_patch_queue.rs` + the CLI's `patches.rs`.
- [`crates/codeless-rpc/src/methods.rs`](../codeless/crates/codeless-rpc/src/methods.rs)
  — args/result shapes for the four new methods.
- [`ui/codeless-ui/src/modules/jobs/StageDetail.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/StageDetail.tsx)
  — adds `ReviewGatePanel` above the existing handover preview for
  REVIEW stages.
- [`ui/codeless-ui/src/modules/jobs/JobTabs.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobTabs.tsx)
  — adds `Patches` to `SystemTabId`, hidden when count is zero.
- New module `ui/codeless-ui/src/modules/patches/` — card component,
  approve/reject hooks, route entry for C.
- [`ui/codeless-ui/src/app/App.tsx`](../codeless/ui/codeless-ui/src/app/App.tsx)
  — `/patches` route + cross-window wiring.
- [`DOCS/SCOPE.md`](./SCOPE.md), [`DOCS/CLAUDE.md`](./CLAUDE.md) —
  seed `enforced_by:` annotations on five rules.

**Auth / R5: unchanged.** Same single-tenant bearer token. No new
permissions surface.

**R1 / R2 / R3 / R4: unchanged.** The new RPCs cross the
RpcClient surface; the UI imports `RpcClient` only (R2). The
predicate runner and process-spawn stay in adapters-host (R1).
There are no per-shell UI files (R3). The mutable artifacts remain
`DOCS/SCOPE-PROPOSED.md` and git commits (R4).
