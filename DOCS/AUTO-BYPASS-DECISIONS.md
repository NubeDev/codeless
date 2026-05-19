# AUTO-BYPASS — recorded decisions

The `auto-bypass-policy` job's
[`SCOPE.md`](../.codeless/jobs/auto-bypass-policy/SCOPE.md) lists six
open questions under "Resolution required from Open questions." Stage 0
of the job is to resolve those questions so every later stage cites a
single source of truth instead of silently re-deciding. This file is
that source of truth. A later stage that contradicts an entry here
without amending it first is a workflow failure.

The deep design these decisions slot into is the workspace
`DOCS/SCOPE-MUTABLE-UI.md` "Surface F" / "Journey 5" / "Dependency #7"
/ "Step 7" sections. The runtime ramp F sits on top of is `Surface E`
in the same doc; its bypass-mechanism decisions land in the
[`SCOPE-MUTABLE-UI-DECISIONS.md`](./SCOPE-MUTABLE-UI-DECISIONS.md) file
already in this repo.

The six questions are taken verbatim from `SCOPE.md`'s "Resolution
required" section. Each carries the doc's lean position; the decisions
below either accept that position or override it, but never leave the
question open.

## Q1 — Thrashing window size

**Decision: two consecutive auto-bypasses with no `Passed` between.**

The doc's lean is two; the decision accepts it.

- A window of **one** turns the policy into a one-strike-out: the
  first failed stage halts, the canned comment never gets a second
  attempt to land, and the operator might as well not have set a
  policy. That defeats the entire surface.
- A window of **three or more** lets a buggy gate burn an additional
  stage's worth of tokens, runner time, and (on REVIEW gates) commit
  noise before the guard fires. The marginal recovery rate of a third
  attempt is not worth the cost; if two canned-comment-guided
  retries did not converge, a third will not either.
- A window of **two** is the smallest number that lets the policy do
  any work at all (one canned-comment-guided retry after the original
  failure) while still capping the cost at a bounded multiple of the
  failure.

**Counting rule.** "Two consecutive auto-bypasses" means two
adjacent `Failed`-then-auto-bypassed stages on the same job with no
intervening `Passed` stage. A `Passed` between them resets the
counter to zero. The counter is per-job, not per-stage-template,
because the same predicate can fail under two different stage names
(e.g. a TYPECHECK gate at stage 3 and again at stage 7).

**State location.** Per
[`SCOPE.md`](../.codeless/jobs/auto-bypass-policy/SCOPE.md)
"Constraints — R4," the count is **ephemeral**: an in-memory map
keyed by `job_id`, rebuilt on driver startup by walking the `stages`
rows for each Running job. No new SQLite column. The reconstruction
is unambiguous because the rule reads off three persisted fields
already on `Stage`: ordinal, status (Failed-with-bypassed-at-set vs
Passed), and the existence of `bypassed_at`.

## Q2 — Policy on cap breach

**Decision: cap breach always halts. Policy is never consulted.**

The doc's prohibition is accepted without qualification. The
`stop_reason` check happens **before** the policy check in the
stage-failed branch:

```text
if stop_reason in (CostCap, WallClockCap):
    halt as today                 # operator's caps win
elif job.auto_bypass_policy is Some(p) and stage.ordinal > 0:
    auto-bypass with p            # policy fires
else:
    halt as today                 # no policy, or stage 0
```

**Pinned failure mode for any future override request.** A future PR
that adds a "let policy override caps" knob is rejected at review
unless it carries a separate decision in this file overriding Q2,
*and* renames the knob so the operator cannot mistake "I want to
auto-bypass on stage failure" for "I want to auto-bypass on cap
breach." Caps are R5's safety net against a runaway runner; the
auto-bypass policy is for failure-class triage. Conflating them
re-opens the bill-shock failure mode that caps were introduced to
close.

If an operator says "the cap is too low for this job," the answer
is **raise the cap**, not bypass the breach. A bypassed cap leaves
no record of the original limit the operator set; a raised cap
does.

## Q3 — Policy on a REVIEW stage that proposed a patch

**Decision: `emit_from_handover` runs first; the auto-bypass branch
runs second. The patch is preserved in `DOCS/SCOPE-PROPOSED.md`
before the gate is bypassed.**

The doc's lean is accepted with the order of operations pinned
explicitly:

1. **Stage handover lands.** The runner exits, the handover string
   is parsed, `raw_tail` is captured.
2. **`emit_from_handover` runs.** Any `SCOPE-PATCH-BEGIN/END` block
   in `raw_tail` is extracted and surfaced via the existing patch
   queue path (Surface B / Surface C plumbing). This step writes
   the proposal artefact to `DOCS/SCOPE-PROPOSED.md` and emits the
   `ScopePatchProposed` event.
3. **Verdict is computed.** `ReviewPreCheck` and `ReviewVerdict`
   events fire as today. If `verdict == FAIL`, the stage is marked
   `Failed`.
4. **Auto-bypass branch runs.** The branch checks the order from Q2
   (cap-breach first, then policy). If the policy fires, the stage
   stays `Failed`-with-`bypassed_at`; the proposed patch from step 2
   is **already** persisted, so the bypass does not lose it.

**Why the order matters.** Reversing steps 2 and 4 would leave a
window in which the auto-bypass advances the job past a REVIEW stage
that emitted a patch the operator never sees. The patch artefact is
the audit trail for what the REVIEW stage *recommended*; the bypass
records what the runtime *did instead*. Both must coexist. The
existing `template_runner` already runs `emit_from_handover` before
the verdict is computed; the auto-bypass branch slots in after the
verdict, which preserves the order without code-shape gymnastics.

**Test obligation.** Stage 5's tests must include a REVIEW-gate
auto-bypass case: a stage whose handover contains a `SCOPE-PATCH`
block, fails its predicate, and is auto-bypassed under a policy.
The assertion is that `DOCS/SCOPE-PROPOSED.md` contains the patch
**after** the bypass advances. A regression here means a silent
loss of an operator's audit trail.

## Q4 — Exact canned comment strings

**Decision: the five strings below are the const &str values.
Stage 2 hard-codes them in `codeless-runtime::auto_bypass_policy`.**

Per `SCOPE.md` "Constraints — Canned comments are `const &str`,"
these are version-controlled in source, not config-file driven, and
not operator-editable at runtime. Changing one is a code change
with a PR. The wording was chosen to:

- name the policy explicitly so the next-stage runner can echo it
  back in its handover (useful for grep-the-log forensics);
- state what the *previous* stage failed at, in the abstract,
  without lying about specifics (the runtime does not know whether
  the failure was a typecheck, a test, or a REVIEW gate);
- give the next stage one sentence of guidance consistent with the
  preset's name, no more.

Each string is a single paragraph, no leading/trailing newline. The
runtime prepends it to the next stage's prompt as an `Operator
comment` block above the goal — same shape as the existing
operator-comment plumbing on `resume_job`'s `comment` argument so
the next-stage runner does not need a new parser.

### Quick

```text
Operator policy: Quick. The previous stage failed and auto-bypass advanced the job. Prefer the smallest change that produces a working result. Skip nice-to-haves; do not refactor surrounding code; do not add new abstractions.
```

### Long-term

```text
Operator policy: Long-term. The previous stage failed and auto-bypass advanced the job. Prefer the durable fix over the quick one. Refactor for clarity if the next change would be harder without it. Tests are not optional; if you change behaviour, change the test that proves it.
```

### Cheap

```text
Operator policy: Cheap. The previous stage failed and auto-bypass advanced the job. Minimise tokens and tool calls. Reuse existing helpers; do not write new infrastructure. If a one-line change unblocks the job, ship it and stop.
```

### Best-judgement

```text
Operator policy: Best judgement. The previous stage failed and auto-bypass advanced the job. The operator is not present to arbitrate quality versus speed. Use your own judgement on the trade-off for this stage; lean on the surrounding code and the project's CLAUDE.md rules to decide.
```

### Just code

```text
Operator policy: Just code. The previous stage failed and auto-bypass advanced the job. The operator wants forward progress. Pick a reasonable approach and ship it; do not block on questions, do not propose a SCOPE patch, do not request review unless the next change is destructive.
```

### Custom (no const)

The `Custom(String)` variant of `AutoBypassPolicy` carries the
operator-supplied free-text directly. The runtime wraps it in the
same `Operator comment` block envelope but does not edit the
contents. The `policy_name` field on `StageAutoBypassed` is the
literal string `"Custom"`; the comment body is whatever the
operator wrote.

**Wording-revision policy.** A typo or word-choice fix here is a
follow-up PR that touches one line in `auto_bypass_policy.rs` and
the matching string in this file. A *meaning* change (e.g. flipping
"do not refactor" to "refactor freely") is a meaningful semantic
revision and goes through the same decisions-doc + REVIEW gate
flow as a new preset would.

## Q5 — `set_job_policy` semantics on a Running job

**Decision: refuse with `RpcError::Conflict`. Operator must
pause-set-resume.**

The doc's lean is accepted. The reasoning:

- A policy change that lands mid-stage races the stage-failed
  handler. Either branch (read-old-policy or read-new-policy) is
  defensible in isolation, but the indeterminacy is not — the
  operator who clicked "change policy" cannot know which branch
  they got. Refusing the change forces the operator's intent to be
  unambiguous before the runtime acts on it.
- Pause is already a primitive the runtime supports (`pause_job`
  RPC, `Paused` status). Resume after a policy change is the same
  resume path operators already use after editing the spec mid-job.
  No new primitive needed.

**Permitted statuses.** `set_job_policy` succeeds when
`job.status` is one of `Draft`, `Stopped`, `Paused`, or
`Completed`. (Completed is allowed because the policy is also
metadata that downstream worklists read; an operator who realises
post-hoc they meant to mark the run as `JustCode` should be able to
correct the record.)

**Rejected statuses.** `Running` and `Queued` are rejected.
`Queued` is rejected for the same reason `Running` is — the
scheduler may transition the job to `Running` between the policy
read and the policy write.

**Error shape.** `RpcError::Conflict(String)` is the existing
variant in `codeless-rpc::error::RpcError`; this RPC reuses it. The
message is one of:

- `"job is Running; pause before changing the auto-bypass policy"`
- `"job is Queued; pause before changing the auto-bypass policy"`

The two messages are distinct so a UI test can assert against the
exact wording without a substring match. Surface F's badge
click-through modal renders the message verbatim under a
"Cannot change now" header.

**Idempotency.** Setting the same policy twice in a row on a
permitted-status job is a no-op success — the second call returns
`Ok(())` and emits no `JobPolicyChanged` event. This keeps
cross-window invalidation traffic to a minimum and means the UI
can call `set_job_policy` defensively without worrying about
event storms.

## Q6 — Cross-doc dependency direction

**Decision: this job assumes the
[`scope-mutable-ui`](../.codeless/jobs/scope-mutable-ui/WORKFLOW.md)
job's Step 1 lands first. If it has not landed when this job's
stage 2 runs, stage 2 adds the `bypassed_at` and `bypassed_reason`
columns inline; the in-flight Surface E branch then reuses them.**

The doc's prohibition on a separate small-PR lift-out is accepted.
The reasoning:

- Surface E's bypass mechanism (`bypassed_at` + `bypassed_reason`
  columns on `stages`, `bypass` arg on `resume_job`,
  resume-skips-Passed semantics) is small enough that lifting it
  out into its own PR adds more coordination cost than it saves.
  The two surfaces share two columns and one event; everything else
  is independent.
- The `scope-mutable-ui` job's Step 1 is already in flight on
  `feat/scope-mutable-ui-step-1` (per
  `SCOPE-MUTABLE-UI-DECISIONS.md` "Scope of this job vs. scope of
  the doc's ramp"). Blocking this job on a hypothetical small-PR
  lift-out would leave Surface F idle while a third PR was scoped,
  reviewed, and merged.

**Operative rule for stage 2.** When stage 2 starts, the agent runs
`git log feat/scope-mutable-ui-step-1 -- crates/codeless-runtime/migrations/`
and inspects the columns on the `stages` table:

- **If `bypassed_at` and `bypassed_reason` already exist** (Surface E
  Step 1 merged to `master` or rebased onto this job's base): stage 2
  reuses them. No migration for those columns; stage 2's migration
  only adds the `auto_bypass_policy` column on `jobs`.
- **If they do not exist** (Surface E Step 1 still in flight):
  stage 2 adds both columns in its own migration. The migration is
  written so it is a no-op if the columns already exist (using
  SQLite's `PRAGMA table_info` check at migration time, not
  `IF NOT EXISTS` which sqlx-migrate does not support uniformly).
  The Surface E branch then drops its column-creation lines on
  rebase.

**Operative rule for stop-the-job.** `SCOPE.md` "Stop-the-job
criteria" lists "stage 2 cannot reuse the columns because Surface E
has not merged" as a halt-and-ask condition. Per this decision,
stage 2 does NOT halt — it adds the columns inline. The
stop-the-job entry is amended in spirit by this decision; if the
ambient situation changes (e.g. Surface E lands in a third
incompatible shape), the agent halts and updates this file before
proceeding.

**Event reuse.** Surface E emits `StageBypassed` for the
operator-clicked path; Surface F emits `StageAutoBypassed` for the
policy-driven path. They are deliberately distinct event variants
so the UI gate panel can render different badges
(`bypassed by operator` vs `bypassed by policy: <name>`). A future
consumer that does not care about the distinction filters on
"either of the two variants" — cheaper than splitting one variant
back into two later.

## Q7 — A "never stops" policy that disables the thrashing guard

**Decision: a seventh preset, `Relentless`, is added. It is the
only `AutoBypassPolicy` variant for which the Q1 two-strike
thrashing guard is skipped. Cost cap and wall-clock cap remain the
only stop signals.**

The operator-driven need is a long-running unattended job whose
only halt conditions are the budget caps. The Q1 guard exists to
prevent silent loops eating budget while the operator is away from
the chair; `Relentless` is the explicit opt-in that says "this is
fine, the caps are the floor, do not halt for thrashing." The
runtime represents this by:

- `AutoBypassPolicy::Relentless` on the wire (kebab-case
  `relentless`), tagged the same way as every other preset.
- `AutoBypassPolicy::thrash_guard_applies()` returns `false` only
  for `Relentless`. `template_runner` reads this flag at the point
  it would otherwise call `ThrashingGuard::would_breach` and skips
  the call entirely; it also skips the matching
  `record_auto_bypass` call so the per-job counter does not grow
  spuriously and confuse a later policy switch.
- The canned comment threaded into the next stage's prompt names
  the policy by its display name and states explicitly that the
  guard is disabled and that the caps are the only stop signal, so
  the model knows the operator's tolerance for forward progress.
- The cost cap and the wall-clock cap continue to halt the job
  unconditionally (Q2 is unchanged). `Relentless` does not weaken
  the budget contract; it only disables the heuristic loop guard.

**Why not "raise the Q1 window to N"?** Because the right behaviour
for the long-running case is "do not halt for repeated bypass at
all" — any finite N has the same shape (silent halt mid-run) for
an N+1 case and is harder to reason about than a single bit. The
operator who picks `Relentless` is signing the consent that the
caps are sufficient.

**Cross-link.** This decision opts out of [Q1](#q1--thrashing-window-size)
for one variant only; the default for every other variant remains
the two-strike window. A future operator who wants a guard size
other than two opens a new Q in this file and bumps the const in
`crates/codeless-runtime/src/thrashing_guard.rs`; they do not flip
this decision.

## Q8 — Infrastructure failures vs the auto-bypass branch

**Decision: a new `FailureClass::InfrastructureError` variant short-
circuits to halt **before** the policy match runs. Caps and infra
share the same Halt branch; the policy is never consulted for either.**

The shipped `auto-bypass-policy` job classified every `RunnerOutcome::
Failed` from a runner subprocess as `FailureClass::RunnerError`,
which the policy treats as "retryable, fire the canned-comment
prompt and advance." Two real jobs (`01KRX4ZPF10J3QZ35R5GK8336X`,
`01KRXH0RYTT6EYGF435WPQS70Q`) completed only because a
`SQLITE_FULL` (sqlx primary code `13`) was bypassed under that
rule. A disk-full / corrupted-DB / IOERR / unreadable-file failure
is not a model-correctable error; retrying the same stage against
the same broken host wastes tokens and noises up the audit trail
without ever converging.

**Error-code set.** The sqlx → `FailureClass` mapper at every
`RunnerOutcome::Failed` site classifies these primary `code()`
values as `InfrastructureError`:

| sqlx primary code | name      | why it is infra              |
| ----------------- | --------- | ---------------------------- |
| `8`               | `READONLY`| host fs / mount is read-only |
| `10`              | `IOERR`   | I/O syscall failed           |
| `11`              | `CORRUPT` | DB file invariant broken     |
| `13`              | `FULL`    | disk or DB page-count cap    |
| `14`              | `CANTOPEN`| DB file unreachable          |

The mapper reads the **primary** code only; extended codes (e.g.
`IOERR_WRITE`, `CORRUPT_VTAB`) collapse to their primary parent.
Code `26` (`NOTADB`) is deliberately **not** in the set — it
indicates a logic bug (the runtime opened the wrong file as a DB)
rather than a host failure, and the right response is a crash with
a stack trace, not a halt with a "infrastructure failure" UI label.

**Layering — why caps and infra share the Halt branch.**
`classify_stage_failure` evaluates, in order:

```text
1. cancel-token tripped       -> Halt (no policy)
2. store unavailable          -> Halt (no policy)
3. row failed to load         -> Halt (no policy)
4. stop_reason.is_some()      -> Halt (caps already won — Q2)
5. failure_class == Infra     -> Halt (Q8 — this entry)
6. job.auto_bypass_policy?    -> Bypass with the matched policy
7. otherwise                  -> Halt (no policy)
```

Steps 4 and 5 are the same posture from two directions: the
operator's caps (Q2) and the host's invariants (Q8) both override
the policy unconditionally. They are the floor the policy cannot
drop below. The Q2 rejection rule applies to Q8 verbatim — a
future PR that adds a "let policy retry infra errors" knob is
rejected unless it overrides Q8 in this file *and* renames the
knob so the operator cannot mistake retry-on-failure for
retry-on-disk-full. A bypassed infra error leaves no record of the
host condition that triggered it, the same way a bypassed cap
leaves no record of the limit.

**Stop-reason stamp.** The Halt branch writes
`StopReason::Infrastructure` on the job row before terminating, so
the UI surface labels the halt panel `infrastructure failure`
rather than falling back to the driver's generic `RunnerCrash`
default. The stamp is warn-only (mirrors `record_thrash_halt`); a
lost stamp degrades the label, not the halt.

**Cross-link.** `Relentless` (Q7) opts out of the thrashing guard
but does **not** opt out of Q8. An infra error halts a Relentless
job. The two are unrelated controls: Q7 disables a heuristic loop
guard, Q8 enforces a hardware-and-fs-invariant floor.

## What is explicitly not decided here

- **Slack integration.** Out of scope for this job per
  [`SCOPE.md`](../.codeless/jobs/auto-bypass-policy/SCOPE.md). The
  matching `@codeless policy` command lives in
  `DOCS/SCOPE-SLACK-INTEGRATION.md` and is a future Slack job's
  problem. The runtime contract this job ships
  (`set_job_policy` RPC, `JobPolicyChanged` event,
  `StageAutoBypassed` event) is the surface the future Slack job
  subscribes to; that is the full contract.
- **Per-stage policy.** Named as an anti-pattern in the doc and
  in `WORKFLOW.md`. No decision needed; it is not on the ramp.
- **A "Skip everything" sixth preset.** Same — named as an
  anti-pattern. The thrashing guard catches the bad case; the
  preset list does not advertise the lazy default.
- **Auto-promotion of the policy preset based on past job
  outcomes.** Out of scope. The operator picks; the runtime does
  not predict.
- **Re-running an auto-bypassed stage retroactively.** Out of
  scope. The audit trail is the audit trail; if the operator wants
  to re-do the work, they submit a new job.
- **A cross-job "policy log" page.** Out of scope. The per-job
  timeline is enough; cross-job aggregation is a separate doc.
- **Implementation specifics.** Wire-type field names beyond what
  Q4 pins, RPC method signatures beyond Q5, component file
  layout. Those land at the stage that ships each surface,
  against the design in the workspace `SCOPE-MUTABLE-UI.md` and
  the rules in `CLAUDE.md`.
