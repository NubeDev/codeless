# SESSION-MUTABLE-SCOPE

A proposal for how Codeless stops fighting rule drift and starts
*compounding* its own rulebook. Not a spec — a thesis to argue with
before anything lands in `SCOPE.md` or `JOB-MODEL.md`.

Read [`SESSION-PEER-REVIEW-IMPROVEMENTS.md`](./SESSION-PEER-REVIEW-IMPROVEMENTS.md)
first. That doc surveys the design space for adding a peer-review
gate between sessions — typed verdicts, reviewer traits, retry state
machines, audit agents, the works. This doc takes the position that
**most of that surface is the wrong abstraction**, and proposes the
shake-up that replaces it.

## Instructions to the reader

Before you read further: **think big.** Do not optimise the doc you
are about to read. Argue with its premises. The point of this
document is to find the framing that makes the next year of Codeless
work compound instead of accumulate. If your reaction is "we could
ship a smaller version of this," you have read it wrong — the smaller
versions are already in the prior doc.

Specifically:

- **Reject incrementalism for its own sake.** "Ship a tier 0 first"
  is correct as a tactic and wrong as a thesis. Tactics serve a
  thesis. If the thesis is wrong, shipping it carefully does not save
  you.
- **Challenge the load-bearing premise.** This doc rests on one claim:
  that the rulebook (SCOPE / JOB-MODEL / CLAUDE.md) should be a
  *system output*, not a *human input*. If that claim is wrong, the
  rest does not matter. Attack it first.
- **Prefer one big change to ten small ones.** Codeless already has a
  stage runner, a template system, an event bus, a SQLite store, and
  a handover protocol. The shake-up is to *use what exists* for a new
  purpose, not to grow a parallel system next to it. If your proposal
  adds a new trait, a new crate, or a new worker pool, you have
  probably reached for the wrong tool.
- **Name what you are willing to throw away.** Every "and also we
  keep X" is a tax on the new idea. The prior doc's P-section
  (typed verdicts, reviewer factories, severity enums, must-address
  lists) is the kind of thing this doc proposes to throw away.
  Throwing things away is the move.

If you finish reading and your instinct is "this is too big" — good.
Push on it until it is the right size. Do not push on it until it is
small.

## The thesis (one paragraph)

Rules rot. CLAUDE.md, SCOPE.md, JOB-MODEL.md are written once by a
human, drift as the codebase moves, and require manual maintenance
to stay accurate. Every proposal in
[`SESSION-PEER-REVIEW-IMPROVEMENTS.md`](./SESSION-PEER-REVIEW-IMPROVEMENTS.md)
— typed verdicts, audit agents, reviewer traits — leaves the rulebook
*static* and adds machinery to enforce it. The shake-up is the
opposite: **make the rulebook mutable, let the system that runs
under it propose its own amendments, and require the amendments to
ship executable checks whenever they tighten the rules.** A REVIEW
stage doesn't only decide whether a WORK stage passed — it decides
whether the rule the WORK stage almost-violated needs strengthening,
emits a patch, and (when tightening) bundles a predicate that makes
the new rule falsifiable from then on. Over weeks the rulebook
sharpens itself on real evidence, with a deterministic floor under
every proposal, instead of rotting between manual edits or ratcheting
on vibes.

## What this replaces

The prior doc's P-section (P1–P10) proposes a peer-review framework:
a `Reviewer` trait, a `ReviewVerdict` enum, a `RedoBriefing` wire
type, retry state machines, crash recovery for in-flight review rows,
a reviewer worker pool. This doc proposes that **all of it collapses
into one sentence**:

> REVIEW is a real stage type. REVIEW stages can amend SCOPE. WORK
> stages cannot.

Everything else falls out:

- "Reviewer trait" → REVIEW is a stage, stages already have a runner
  abstraction.
- "ReviewVerdict enum" → REVIEW stage output is a PASS/FAIL sentinel
  line plus an optional structured SCOPE patch. No new wire type.
- "Retry state machine" → if you want one redo, the template includes
  a `REDO-WORK` stage after `REVIEW`. Template syntax already exists.
- "Reviewer worker pool" → stages already run in the runner pool.
- "Crash recovery for in-flight reviews" → stage crash recovery
  already exists.
- "Reviewer-error vs review-fail distinction" → stage-error vs
  stage-fail distinction already exists.

The prior doc designs a parallel system. This doc reuses the system
that exists.

## The loop

```
WORK stage:
  Reads SCOPE (read-only) + the predicate list.
  Does the work.
  Writes handover.

[Runtime, no model] — Layer 1 pre-checks, in order:
  - diff-verify: every path in handover.done appears in git diff.
  - predicate run: every checked-in predicate exits 0 on the diff.
  Any failure short-circuits to FAIL with no tokens spent.

REVIEW stage (only if pre-checks pass):
  Reads SCOPE + diff + handover + pre-check report.
  Emits:
    1. PASS or FAIL sentinel.
    2. (optional) a structured SCOPE patch, one of:
         - prose-only (add or loosen a rule)
         - prose + predicate (tighten a rule, ship the check)
         - predicate-only (existing rule is fine, just enforce it)

TEST stage:
  Runs tests.
  On failure, decides:
    - flaky/broken test → FAIL the stage.
    - test reveals an invariant SCOPE didn't name → propose a SCOPE
      patch (ideally with a predicate derived from the failing
      assertion) + PASS.
```

The reviewer's job is not "did this stage pass." It is "did this
stage reveal a rule that needs strengthening, and can I ship the
check that makes the new rule falsifiable." Every FAIL is also a
dataset point about which rule was too soft. Every PASS that
*almost* failed is a dataset point about which rule is *barely*
holding — and a candidate for promotion from prose to predicate.

## The three layers: Rust, docs, AI

The boundary between layers is the load-bearing engineering decision.
Get it wrong and you either build a Rust framework around vibes, ship
a prompt that hand-waves at security, or pile prose rules nothing
checks. There are three layers, not two — treating "docs" as a real
layer (not just the seam between Rust and AI) is what stops rules from
rotting.

### Layer 1 — Rust (deterministic, falsifiable, security-shaped)

Hardcoded checks. Run in milliseconds, cost zero tokens, never wrong
about the thing they check. The cost of a false positive is bounded
(auto-FAIL, human re-runs) and the check is a pure function of the
diff + repo state.

- Who can write to SCOPE.md. Runtime refuses to commit a WORK stage
  diff that touches SCOPE.md or any file in `DOCS/` listed as
  rule-bearing.
- Stage state machine. WORK must be followed by REVIEW before the
  next WORK runs. Template validator rejects WORK→WORK.
- Diff-verify pre-check. Every path the handover's `done` mentions
  must appear in `git diff`. Auto-FAIL before the REVIEW prompt runs.
  Highest-signal check in the proposal; never sees a model.
- SCOPE patch format. Patches are structured (small YAML/JSON shape:
  `target_rule_id`, `current_text`, `proposed_text`,
  `evidence_stage_id`, `rationale`, optional `predicate_path`).
  Not free-prose markdown surgery.
- PASS/FAIL sentinel grammar. One line, one of two values, on the
  last non-empty line of the REVIEW output. Anything else is a parse
  failure. Parse failure blocks advance.
- Predicate runner. The `xtask` (or equivalent) that executes the
  checked-in executable predicates (next section). Pure Rust /
  shell, deterministic, exit-code-shaped.

### Layer 2 — docs / md (rules humans and models both read)

The prose rulebook (`SCOPE.md`, `JOB-MODEL.md`, `CLAUDE.md`) plus the
REVIEW / WORK prompt templates. Read by every WORK stage as context,
read by every REVIEW stage as the spec it enforces against. The seam
between deterministic and judgement.

- Rule wording. The rules themselves. Belong here if they need a
  paragraph of context to evaluate, or if their primary value is
  being *read before the work*, not enforced after.
- Worked examples and anti-examples. Catch more drift than any
  validator because the model never has to interpret them.
- Prompt templates for WORK / REVIEW / TEST stages. Iterable in
  days, not weeks. Versioned in git like any other rule.
- The proposed-patch queue (`DOCS/SCOPE-PROPOSED.md`). Human-readable,
  human-editable, human-approvable. The editor's inbox.

### Layer 3 — AI / prompts (judgement, context, cheap to iterate)

The model's job. Cost: tokens per check, non-determinism, harder to
audit. Reach for AI only when neither a Rust check nor a prose rule
read upfront can do the job.

- "Rule was followed in letter but not in spirit" — wording-loose
  drift detection.
- Whether a test failure reveals a missing invariant or just a bad
  test.
- The diff review itself — does the code do what the stage title
  said it would.
- What the SCOPE patch should *say* — the proposed wording and the
  rationale.
- Whether a near-miss is worth proposing a patch for, or whether the
  existing rule is fine and the worker just needed to read it more
  carefully.
- Whether a Layer-2 rule has matured enough to be promoted to a
  Layer-1 executable predicate.

### Litmus tests for picking a layer

| Question | Layer |
|----------|-------|
| Can a unit test decide it from diff + tree? | Rust |
| Does it need a paragraph of context to make sense? | docs |
| Is the rule most useful *read before* the work? | docs |
| Is it "did the spirit hold," not "did the letter"? | AI |
| Is the judgement itself the interesting output? | AI |
| Is it cheap to run and we want to run it every stage? | Rust |
| Will a brand-new agent reading it cold need the why? | docs |

The failure modes are symmetrical:

- **Rust over-reach.** Encoding judgement as a boolean. The rule
  never matches the situation; the worker games it; the team learns
  to ignore the auto-FAILs.
- **Docs over-reach.** Rules that should be executable predicates
  rot because nothing actually checks them. CLAUDE.md today is the
  cautionary tale.
- **AI over-reach.** Every check costs tokens, every check is
  non-deterministic, every check is unauditable. Reviews that should
  have been `cargo check` cost real money and miss real bugs.

The directional default: **start in docs, promote to Rust when a rule
proves it deserves an executable form, fall back to AI only for the
residue neither layer can absorb.** Most rules never need to leave
docs. The rules that do are the ones the team got bitten by twice.

## Executable predicates: the bridge

The thing that makes the three-layer model compound, rather than just
co-exist, is a Layer-2 → Layer-1 promotion path. Predicates are it.

A predicate is a small checked-in probe — a `cargo xtask` task, a
`scripts/check_*.sh`, a tree-sitter query — that returns exit code 0
(rule held) or non-zero (rule violated, with citations). Predicates
live alongside the prose rule they enforce:

```
DOCS/SCOPE.md                 # rule: "API handlers must have a #[tracing::instrument]"
crates/codeless-xtask/src/check_handler_instrumentation.rs
                              # predicate that enforces it
```

A SCOPE patch can propose four kinds of change. Each direction has a
matching falsifiability requirement; without it, the AI takes the
unfalsifiable path every time.

1. **Add a rule (prose-only).** "There should be a rule R about X."
   Adding a brand-new rule is allowed prose-only because a new rule
   cannot cascade — no prior stage was relying on it. The patch must
   still cite an evidence stage that motivates the rule.
2. **Tighten a rule (prose + predicate).** "Rule R was followed in
   spirit but the wording let the worker drift; tighten it to R'."
   Must ship either a new predicate or a reference to an existing
   predicate it sharpens. **Tightening without a predicate is
   rejected at parse.**
3. **Loosen a rule (prose + counter-evidence).** "Rule R is over-broad;
   loosen it to R'." Must cite an evidence stage where a *legitimate*
   change was blocked by the current rule, *and* must add a
   positive-case fixture to the predicate's tests showing the new
   behaviour now passes. **Loosening without a positive fixture is
   rejected at parse.** This is the symmetric guard against the AI
   discovering "loosen the rule" as the easy path out of every
   contested predicate.
4. **Enforce an unchecked rule (predicate-only).** "Existing rule R
   is fine; we just never had a check for it. Here's one." Pure
   Layer-1 hardening, no wording change.

The two parse-rejections are the load-bearing structural rules:

- *Tightening requires a predicate* prevents hallucinated rules from
  landing — the predicate fails on the next CI run if the rule is
  fiction.
- *Loosening requires a positive fixture* prevents the easy escape
  hatch from every contested predicate — the AI cannot quietly
  defang Layer 1 by repeatedly loosening rules that work.

This is what makes mutable-SCOPE worth shipping rather than just
doing tighter prose. Prose-only patches accumulate; predicate
patches *compound* — every approved patch leaves the runtime
strictly more capable of catching its own class of drift, with no
ongoing token cost.

### One patch per REVIEW stage

By parser fiat: a single REVIEW stage emits at most one structured
patch. A stage that genuinely surfaces multiple drift signals must
pick one and drop the rest; the dropped observations are visible in
the REVIEW transcript but do not become patches.

The cost is real (information loss). The win is bounded patch volume:
patch throughput is capped at `stages/day`, not `stages/day × n`,
which is what keeps the human editor's workload tractable. Without
this cap, every "and also we noticed X" multiplies the queue.

### How patches earn their layer

| Patch shape | What lands |
|-------------|------------|
| Prose-only | New / changed wording in `SCOPE.md` |
| Prose + predicate | Wording change + new predicate + test for the predicate |
| Predicate-only | New predicate + the existing rule annotated `enforced_by: predicate_path` |

A rule annotated `enforced_by:` is a rule that has earned Layer-1
status. Over time, the rulebook stratifies: rules with predicates are
load-bearing; rules without are advisory. That stratification is the
artefact — it tells a new agent which rules are *really* enforced and
which are still aspirational.

## What is mutable, what is not

The mutable rulebook is not the same set as "all files in `DOCS/`."
Treating it that way is the most plausible way to break the user
contract by accident.

**Mutable on REVIEW-patch terms:**

- `DOCS/SCOPE.md` — product scope, what Codeless is.
- `DOCS/CLAUDE.md` — agent rules, how to work here.
- Predicate files under the predicate-runner crate.

**Not mutable — wire formats, changed only by explicit versioning:**

- `DOCS/JOB-MODEL.md` — the user-facing job / handover contract.
  External tooling, user habits, and the runtime's parser all
  couple to its shape. A REVIEW stage that "tightens" the handover
  schema can break every prior job's stored handover silently.
- `DOCS/JOB-LOOP.md` — the tick procedure CLAUDE.md leans on. A
  drift here corrupts the loop itself.
- `codeless-types/src/handover.rs` and any other wire type. These
  change via `schema_version` bumps and migrations, never via
  REVIEW patches.

The split is enforceable: the patch parser knows the mutable set; a
patch whose `target_file` is outside the mutable set is rejected at
parse, no model invoked. Wire formats stay sacred; rulebooks
compound.

## Why this is the shake-up

The current world:

```
Rules live in CLAUDE.md.
Agents read them.
Agents drift.
Human notices the drift weeks later.
Human hand-edits CLAUDE.md from memory of recent failures.
SCOPE rots between manual edits.
Repeat forever.
```

The new world:

```
Rules live across three substrates:
  Layer 1 — Rust runtime + executable predicates (deterministic).
  Layer 2 — SCOPE.md / CLAUDE.md prose (read by humans and models).
  Layer 3 — REVIEW prompts (judgement on what the other two miss).
WORK agents read Layer 2 and run under Layer 1.
Pre-checks (Layer 1) catch the falsifiable failures with no tokens.
REVIEW agents (Layer 3) judge the residue, and when they notice
  drift they propose a patch — ideally one that ships its own
  predicate so the rule moves down to Layer 1.
Human becomes the merge-reviewer for patches, not the author.
The rulebook stratifies on real evidence: rules earn Layer-1
  status by being enforced in code; the prose layer shrinks toward
  the irreducibly contextual.
```

The human's role shifts from *author* to *editor*. The rulebook's
shape shifts from "prose that rots" to "predicates that compound."
Reviewing a three-line diff plus a predicate is a fundamentally
different job than rewriting a rule from memory. The author job
does not scale; the editor job does; the predicate is what makes
the editor's approvals *stick* — once merged, no agent ever has to
re-read the rule for it to fire.

## The risk this doc owes you

SCOPE-as-mutable means SCOPE-as-attack-surface. A REVIEW stage that
hallucinates a rule and tightens SCOPE to forbid something legitimate
will cascade — the next WORK reads the broken rule, fails, the next
REVIEW tightens further, and the rulebook drifts *away* from reality
under its own momentum. This is the failure mode that kills the idea
if you ignore it.

Mitigations, in order of how much they cost:

1. **Tightening patches must ship a predicate** (Layer 1, parse-time).
   A hallucinated rule cannot land because its predicate would fail
   immediately against the live tree. Predicates fail loudly in CI;
   vibes do not.
2. **Loosening patches must ship a positive fixture** (Layer 1,
   parse-time). Symmetric to the tightening guard. Closes the easy
   path where the AI defangs Layer 1 by repeatedly loosening rules
   that work. Without this, "the rule is wrong" is unfalsifiable in
   the same way "the rule should be tighter" would be without the
   tightening guard.
3. **One patch per REVIEW stage** (Layer 1, parse-time). Bounds
   patch volume at `stages/day`. Without this, a single REVIEW
   stage can saturate the human approval queue.
4. **Patches do not auto-apply, ever** (Layer 2). REVIEW writes
   proposals to `DOCS/SCOPE-PROPOSED.md`. A human approves them in
   batches. There is no auto-merge mode — see the ramp's
   "deliberately not included" section for why.
5. **Patches must cite evidence** (Layer 1, parse-time). A patch
   with no `evidence_stage_id` is rejected. A patch citing an
   evidence stage whose diff does not contain the cited behaviour
   is rejected. Abstract "rules should be tighter" patches are
   impossible to express.
6. **Patches are reversible.** They are commits. If a patch turns
   out wrong, revert. The runtime can correlate "stages started
   failing after SCOPE@\<sha\>" and surface that as a signal — the
   rulebook itself is subject to the gate it enforces.

Note how the layering reshapes the risk: in a pure-AI version of
this doc, every mitigation is procedural (humans review, evidence
cited, reversible). With Layer 1 holding the line at the bottom —
*and on both directions*, tightening and loosening — a bad patch is
a parse rejection or a CI-red commit, not a slow-rotting rulebook.
That is the difference between "we manage the risk" and "the risk
has a deterministic floor on both sides."

## What ships, in order

A ramp, not a tier list. Each step is independently useful and each
step makes the next step cheap. The ramp is organised by layer: docs
first (cheapest, least to throw away), then Rust gates (deterministic
wins), then AI judgement (where tokens earn their keep), then the
predicate substrate that lets the rulebook compound.

### Step 0 — Docs only, ship today (hours, Layer 2)

Zero code changes. The worker just gets a better spec to read.

- Tighten the handover spec in `JOB-MODEL.md` with a worked example
  and an anti-example per section.
- Add a JOB-LOOP rule: ack-then-code. A stage that received a
  prefixed handover restates the prior `next` in its own words in
  the first reply.
- Add a JOB-LOOP rule: verify-before-handover. Before writing the
  handover, run `git diff <stage-base>..HEAD` and confirm every
  path mentioned in `done` appears in the diff.

These cost nothing and catch the most common drift modes before any
of the runtime work below is needed.

### Step 1 — REVIEW as a real stage type (Layer 1)

PASS/FAIL only. No SCOPE patch output yet. The template runner gates
advancement on REVIEW passing.

Two file-set rules land in Layer 1 here:

- **WORK stages cannot touch any rule-bearing file.** The list lives
  in `codeless-runtime` config: `DOCS/SCOPE.md`, `DOCS/CLAUDE.md`,
  `DOCS/JOB-MODEL.md`, `DOCS/JOB-LOOP.md`, and any predicate file
  under the predicate-runner crate. WORK touching any of these fails
  the commit, runtime-enforced.
- **REVIEW patches can target only the *mutable* subset of those.**
  Mutable: `DOCS/SCOPE.md`, `DOCS/CLAUDE.md`, predicate files.
  Not mutable: `DOCS/JOB-MODEL.md`, `DOCS/JOB-LOOP.md`, the handover
  schema in `codeless-types/src/handover.rs`. Those are the
  user-facing wire format — they change via explicit versioning,
  not via REVIEW-stage patches (see "What is mutable, what is
  not" below).

Note on naming: `Event::ReviewRequested` already exists in the
runtime today as an advisory event emitted by REVIEW-prefixed
stages. The blocking-gate REVIEW stage proposed here either replaces
that event's semantics or needs a distinct name (`Event::ReviewGate*`).
Decide before step 1 lands; do not let two REVIEW concepts coexist
silently.

### Step 2 — Diff-verify pre-check (Layer 1)

Before the REVIEW stage's prompt runs, the runtime walks every path
mentioned in the handover's `done` and checks it appears in `git
diff`. A miss is an automatic FAIL with no model invoked.
Deterministic, free, and catches the most common worker failure mode.
Highest EV in the entire ramp.

### Step 3 — Predicate runner, seeded with hand-written probes (Layer 1)

Stand up the `xtask` (or equivalent) that runs checked-in predicates
on every stage's diff. Ship it with 3–5 hand-written predicates that
encode rules the team already cares about (e.g. "no `tokio::process`
outside `codeless-adapters-host`" — already in SCOPE, currently
unchecked). Predicate failure is a Layer-1 auto-FAIL with no model
invoked, same as diff-verify.

This step does *not* depend on mutable SCOPE. It is a standalone win:
the rulebook gets executable teeth even if no patch is ever
auto-proposed.

### Step 4 — SCOPE patch output, shadow mode (Layer 3 + 2)

REVIEW stages start emitting structured patch proposals to
`DOCS/SCOPE-PROPOSED.md`. Nothing merges automatically. The
calibration phase: read the proposals, decide whether they are
useful. **Kill criterion: if more than 60% are noise after four
weeks, abandon the auto-proposal path** and keep only steps 0–3.

### Step 5 — Patch shape rules enforced at parse (Layer 1 over Layer 3)

The structural guards from "Executable predicates: the bridge" land
in the parser:

- Tightening patches must ship a predicate (or reference one they
  sharpen).
- Loosening patches must add a positive fixture to the predicate's
  tests *and* cite an evidence stage where legitimate code was
  blocked.
- One patch per REVIEW stage.

This is the step that turns mutable-SCOPE from "vibes ratcheting
prose" into "compounding executable rulebook." It is also the step
that makes the risk section actually small: hallucinated rules can't
land (failing predicate), AI can't quietly defang Layer 1 by
loosening (missing positive fixture), and patch volume is bounded.

### Step 6 — Patch approval UX (Layer 2)

A small UI affordance (or CLI command) to walk proposed patches,
approve / reject / edit. Approved patches land as normal commits,
authored by the human, with the evidence stage cited in the commit
body. The predicate files land in the same commit.

The ramp ends here. The two further steps the earlier draft listed
(TEST stages emitting patches; auto-merge after a delay window) are
deliberately dropped — see "What this ramp deliberately does not
include" below.

### Stopping points

- Stop at step 0: you have shipped a better spec. Free.
- Stop at step 2: you have shipped the gate. Most of the
  prior doc's P-section value.
- Stop at step 3: you have shipped executable rules. The rulebook
  has teeth even without mutability.
- Stop at step 5: parse-time guards are in place. Even if step 6
  slips, proposed patches are well-shaped enough to read in a flat
  markdown file.
- Reach step 6: you have shipped a *compounding* rulebook with a
  human-in-the-loop approval gate. The system is sustainable here.

Each stopping point is a win. Step 5 is where the cascade risk
becomes structurally bounded; step 6 is where the human's editor
role becomes ergonomic. The earlier draft's ambition to reach
"self-modifying without supervision" has been intentionally trimmed.

### What this ramp deliberately does not include

- **TEST stages proposing patches.** A failing test tells you exactly
  one thing: this assertion did not hold under these inputs. The leap
  from there to "and the underlying invariant deserves a project-wide
  rule" is unbounded inference under the most ambiguous evidence the
  system ever sees. Realistic failure mode: a property test catches
  an off-by-one in a date helper; TEST proposes "all date arithmetic
  must use UTC" with a `grep chrono::Local` predicate; predicate
  passes on trunk (nothing currently uses it); rule lands; three
  months later a legitimate timezone-aware feature can't ship without
  fighting it. TEST stages flag failures for *human triage*; humans
  decide whether the failure surfaces a rule gap. The "rulebook
  compounds" property comes from REVIEW + predicate, not TEST +
  predicate.
- **Auto-merge with a delay window.** Predicate-passes-on-trunk is
  necessary, not sufficient. It does not catch rule-scope mistakes
  (rule applies project-wide but evidence was domain-specific),
  ambiguous wording that future REVIEW stages will enforce
  inconsistently, or emergent overconstraint (each rule individually
  fine, the set is not). None of these surface inside a 24h window;
  they surface at the next contested WORK stage. A "kill-switch
  available" disclaimer is doing more work than it can — by the time
  you flip it, you have absorbed a week of unreviewed rule changes.
  If steps 4–6 succeed and patch volume is low, the marginal cost of
  the human clicking *approve* is small. If patch volume is high
  enough that auto-merge would help, that itself is the signal that
  the patches are not high enough signal to merge unattended.

These two omissions are the line between this product and a different,
strictly more dangerous product. Crossing the line is an explicit
re-decision, not a continuation of this ramp.

## What lands where in the codebase

The "reuses what exists" claim is only auditable if you can point at
the files. The touch points, grounded against the current crate
layout:

- [`template_runner.rs`](../codeless/crates/codeless-runtime/src/template_runner.rs)
  gets the new stage type and the Layer-1 pre-check phase
  (diff-verify + predicate runner). This is where steps 1–3 of the
  ramp land. The advancement gate keys off the PASS/FAIL sentinel.
- [`handover.rs`](../codeless/crates/codeless-types/src/handover.rs)
  does *not* change. Patches are not handover. A new wire type
  `ScopePatch` lives next to it in `codeless-types`, mobile-safe,
  no process spawn.
- The patch parser lives in `codeless-runtime` (parses the structured
  patch out of REVIEW output, applies the mutable-set / shape-rule
  checks). Pure Rust, no model.
- The predicate runner is a new crate — likely `codeless-xtask` or
  a sibling — that depends on `codeless-adapters-host` transitively
  for shell-out probes. **Unreachable from the mobile build** per
  R1 (crate dependency direction). Predicate files live under that
  crate.
- The proposed-patch queue is a plain file in the user's repo at
  `DOCS/SCOPE-PROPOSED.md`. No new runtime data path. Committed and
  versioned like any other doc.
- The mutable-set / wire-format-set lists live in
  `codeless-runtime` config — small static lists, not user-editable
  at runtime.
- **Auth / R5: unchanged.** The runtime that writes
  `SCOPE-PROPOSED.md` is the same runtime that writes `handover.md`.
  Same bearer-token trust boundary, same single-tenant model. No
  new permissions, no new trust surface — calling this out so
  reviewers do not have to reverse-engineer it.

## Dependencies on the prior doc that do not collapse into this one

The prior doc's P-section collapses into "REVIEW is a stage." Its
H-section does *not* collapse — those are correctness gaps in the
handover system that exist independently of whether SCOPE becomes
mutable:

- H1 (per-stage handover, not just per-job).
- H3 (keyed handover discovery, not mtime-ranked).
- H7 (write-time validation: non-empty `done` / `next`).

The peer-review gate proposed here can be built on top of the
mtime-based handover discovery the runtime has today, but it is
strictly more brittle until H1 / H3 / H7 land. Ship them in the same
window as steps 0–2 of this ramp, not after.

## What this doc deliberately does not address

- **Template syntax for REVIEW/TEST stages.** Out of scope here. The
  template system already supports stage types; adding two more is
  bookkeeping, not design.
- **The exact REVIEW prompt.** The prompt is the product, but it is
  iterable in days, not weeks. Do not block the architecture on
  prompt wording.
- **Different-runner reviewers (claude work / codex review).** Worth
  doing, captured in the prior doc as P5. Orthogonal to this doc —
  works the same whether the REVIEW stage runs claude or codex.
- **Multi-tenant, per-user permissions, audit logs for compliance.**
  R5 (single-tenant trust boundary) still holds. The mutable
  rulebook does not change the trust model.
- **TEST stages proposing patches; auto-merge.** Covered in the
  ramp's "deliberately not included" section, with reasoning.

## Open questions worth fighting about

- Should the WORK stage be allowed to *read* a proposed-but-not-yet-
  approved SCOPE patch? Argument for: the worker sees the rule that
  is about to be enforced, gets a chance to comply early. Argument
  against: the rulebook the worker reads should match the rulebook
  the reviewer enforces, full stop. Probably: against.
- Should there be a `RULE-DEPRECATION` patch type, or is removing a
  rule the same operation as adding one? Probably: same operation,
  one shape — and loosening / deletion patches are the one case
  where prose-only is fine, since a looser rule cannot cascade into
  the failure mode the predicate requirement guards against.
- Does a SCOPE patch that strengthens a rule re-trigger review of
  *prior* stages that would have failed under the new rule? Argument
  for: rigour. Argument against: the loop never terminates. Probably:
  against — rules apply forward from their merge commit, period. A
  newly-added predicate, by contrast, *does* run against current
  trunk on the next CI tick, which surfaces accumulated debt
  organically without re-litigating closed stages.
- Where do predicates live in the crate graph? R1 (crate dependency
  direction) implies they cannot live in any mobile-safe crate if
  they shell out. Probably: an `xtask`-shaped crate that depends on
  host adapters and is unreachable from the mobile build.
- What's the lifecycle for a predicate that goes stale (e.g. checks
  a directory that was renamed)? A failing predicate the team agrees
  is obsolete needs a deletion path that does *not* route through
  "WORK stage edits SCOPE.md" (forbidden by Layer 1). Probably:
  predicate deletion is its own patch type, same approval gate.
- How aggressive should the runtime be at suggesting prose →
  predicate promotion? A rule cited as evidence in N approved patches
  is a strong candidate. Probably: surface the candidates as
  suggestions in the patch UI; never auto-promote, because writing
  the predicate is itself a real code change.

## Pointer

- Prior doc this one replaces:
  [`SESSION-PEER-REVIEW-IMPROVEMENTS.md`](./SESSION-PEER-REVIEW-IMPROVEMENTS.md)
- The rulebook this doc proposes to make mutable:
  [`SCOPE.md`](./SCOPE.md), [`JOB-MODEL.md`](./JOB-MODEL.md),
  [`JOB-LOOP.md`](./JOB-LOOP.md)
- The stage runner this doc proposes to reuse:
  [`template_runner.rs`](../codeless/crates/codeless-runtime/src/template_runner.rs)
- The handover contract REVIEW stages read against:
  [`handover.rs`](../codeless/crates/codeless-types/src/handover.rs)
