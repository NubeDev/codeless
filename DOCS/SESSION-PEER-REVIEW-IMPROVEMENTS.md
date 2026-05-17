# SESSION-PEER-REVIEW-IMPROVEMENTS

Design notes for evolving Codeless's session-handover model and adding
a peer-review gate between sessions. Not a spec — a proposal to argue
with before anything lands in `SCOPE.md` or `JOB-MODEL.md`.

Related:
- [`JOB-MODEL.md`](./JOB-MODEL.md) — current handover contract
- [`JOB-LOOP.md`](./JOB-LOOP.md) — tick procedure
- [`codeless-types/src/handover.rs`](../codeless/crates/codeless-types/src/handover.rs) — the wire type
- [`codeless-runtime/src/handover.rs`](../codeless/crates/codeless-runtime/src/handover.rs) — write/read/extract
- [`codeless-runtime/src/template_runner.rs`](../codeless/crates/codeless-runtime/src/template_runner.rs) — multi-stage execution
- [`AGENT.md`](./AGENT.md) — persona / subagent / runner split; per-stage persona override is the natural companion to the per-stage handover and reviewer-as-separate-session ideas below

## What the system does today (one paragraph)

A session ends; `ClaudeRunnerAdapter` looks at the assistant's final
text, extracts a fenced ` ```handover ` block, and writes it to
`<worktree>/runs/<job_id>/handover.md` with four canonical sections
(`Done` / `Next` / `What you need to know` / `Open questions`). When
the next job is dequeued, `job_driver_loop` calls
`find_latest_handover` (newest-by-mtime) and prepends
`prompt_prefix_for(...)` to the new job's prompt. Inside a single
multi-stage job, `TemplateRunner` does **not** write or read handovers
between stages — state passes via the git worktree and (optionally) a
resumed claude session id. There is no machine-checked summary, no
review gate, no retry policy.

## Does a session already leave a summary?

Yes, but with three real limitations:

1. **One handover per job, not per stage.** A 9-stage job writes one
   `handover.md` at the end of stage 9. If stage 4 of 9 was the
   important inflection point, that decision lives in the model's
   in-context memory or the commit message — not in a structured doc.
2. **Self-reported.** The model that did the work writes its own
   report card. There is no second pair of eyes between "I think I'm
   done" and "the next session believes me."
3. **Mtime-keyed discovery.** `find_latest_handover` walks
   `.codeless/worktrees/job-*/runs/*/handover.md` and picks the newest
   file by mtime. Concurrent jobs in the same repo can race; the
   "wrong" job's handover can prime the wrong next session. This is
   the failure mode H3 below targets — referenced from there rather
   than re-described.

## Handover changes

Split into **correctness gaps** (H1, H6, H7 — the current system gets
these wrong, they aren't 10x improvements) and **leverage moves** (H2,
H3, H4, H5, H8 — these change what the system can do). The MVP slice
at the bottom favours the correctness gaps; leverage moves layer on
top.

### H1. Per-stage handover, not just per-job

Write a `Handover` at the end of every stage to
`runs/<job_id>/<stage_id>/handover.md`. The next stage's
`stage_prompt` prepends the prior stage's handover the same way
`job_driver_loop` prepends the prior job's. Hook point is the loop tail
in `TemplateRunner::run` right after `RunnerOutcome::Completed`. This
turns the four-section contract into a real cross-stage protocol
instead of a job-terminator artefact. Cost: handover discovery has to
change from "newest mtime" to "named by `(job_id, stage_id)` lookup".

### H2. Structured handover beats prose handover

Today `Handover` is `Vec<String>` per section — the model writes free
text and the runtime parses bullet lines. Push more structure where it
exists:

- `decisions: Vec<Decision { what, why, alternative_considered }>`
- `invariants: Vec<Invariant { statement, enforced_by }>`
- `followups: Vec<Followup { description, owner_stage, blocking: bool }>`
- `commit: Option<CommitRef { sha, branch }>` — the stage's commit, if
  any.

**Rule: use `git diff` for the file list, do not duplicate it in the
handover.** The handover stores the commit SHA; the next session (and
the reviewer) runs `git diff <sha>^..<sha>` to see what changed.
Storing a separate `artefacts` array creates a second source of truth
that will drift from the actual tree the moment something rebases,
amends, or cherry-picks. One source of truth per fact: commits own
"what files changed", handover owns "what the session was thinking".

`done` / `next` / `what_you_need_to_know` stay as free prose for the
soft stuff; the structured fields make the handover queryable,
diff-able, and trustable. The UI's Handover panel becomes a real
artefact view, not a markdown render.

### H3. Handover discovery is keyed, not heuristic

Replace mtime ranking with explicit lookup: the next session asks for
`(job_id, stage_id - 1)` or `(repo_id, prior_job_id)`. Delete
`find_latest_handover` rather than keeping it as a fallback — an
unkeyed fallback is exactly the race condition this fix is meant to
remove. If a session has no key to look up, that is a bug in the
caller, not a case the runtime should paper over.

### H4. Handover is published as an event, not just a file

`Event::HandoverWritten { job_id, stage_id, path, summary_sha }` flows
through the same event bus the StageRecorder consumes. The UI can
subscribe and badge the stage. The peer-review gate (below)
subscribes too instead of polling the filesystem. The file remains
the source of truth; the event is the notification.

### H5. Round-trip the handover through the model's *next* turn

When a new session starts with a prepended handover, ask it to
**acknowledge** ("ack" = a structured first-reply summary) the four
sections. The runtime parses the ack and stores it. The temptation
is to NLI-check the ack against the handover prose, but free-prose
NLI is noisy enough that the false-positive rate trains the team to
ignore the flag. Gate the check on the *structured* fields from H2
instead: does the ack reference each `Decision` by id? Does it list
every `Followup` marked `blocking`? A missing structured field is a
hard signal; a paraphrased prose section is not. This keeps the
catch-rate for "model ignored the prefix and re-did stage 4" without
the NLI noise floor.

### H6. Versioned handover schema

Stamp `schema_version: u32` on the wire type now while it is cheap. A
session that reads a newer-than-supported handover should fail closed
instead of silently dropping unknown fields, and the system prompt
hint can vary by version. "Fail closed" means: the runtime parks the
job in `AwaitingHumanReview` with `Event::HandoverSchemaMismatch
{ found, supported }`; it does not hard-fail the runtime and does not
silently downgrade. The version mismatch is a deployment problem, not
a job problem.

### H7. Handover validation at write time

Right now a blank `done` is just discouraged in JOB-MODEL.md prose.
Promote the rule into `Handover::validate()`:

- `done` non-empty unless `status == Aborted`
- `next` non-empty unless `status == Completed` AND this was the last
  stage of a finite plan
- `what_you_need_to_know` non-empty if the stage made any
  non-trivial-shaped commit. Threshold TBD: pick a number once we have
  a week of real handover data to look at — do not ship `N=1` as a
  placeholder. Until the threshold is set, this rule is a warning,
  not a validation failure.
- `open_questions` empty implies *settled* — the next session reads it
  as "do not re-litigate"

`extract_handover` returns `Result<Handover, ValidationError>` and the
fallback path is reserved for genuine parse failures, not "the model
emitted a blank section".

### H8. Handover-as-prompt-budget

A handover that grows unboundedly across many stages is the slow
version of "session prompt overflows". Today `prompt_prefix_for` just
concatenates. Add a budget per session run: each handover gets a hard
ceiling (e.g. 8k tokens). Keep both copies on disk:
`handover.full.md` (uncompressed, audit trail) and `handover.md`
(compressed, what the next session prompt prefix actually sees).
Compressing destructively would mean a future debugger can never see
what the model originally wrote — that's the wrong tradeoff for a few
extra KB of disk.

## Peer review between sessions

The user's framing: a session, before "moving on", invokes a peer
review. The reviewer decides whether the session can hand off cleanly,
or whether a redo is required. One redo, then a fresh peer review. If
that fails too, the session escalates rather than spinning.

This maps onto existing pieces: REVIEW stages already exist in
`TemplateRunner` (they emit `Event::ReviewRequested` but do not block
today). The peer-review proposal turns the gate from advisory into
load-bearing.

**Load-bearing premise (carried by every P-idea below): the reviewer
runs in a separate session from the worker.** Same-session
"self-review" is not a review — it is a self-confirmation. The whole
gate degrades to theatre if this is relaxed. P1 spells out the
mechanics; the rest of the P ideas assume it.

### P1. Reviewer is a separate session, not the same model thread

The session that did the work is not the session that reviews it. A
peer reviewer:

- Starts fresh (no inherited claude session id).
- Sees only the handover, the diff, and the job goal / stage title.
- Has no write tools — only read + a structured `review.decide` RPC.

This is what makes it a *review* rather than a self-confirmation. If
the reviewer can edit, the gate is theatre.

### P2. Pass / fail is a typed verdict, not prose

```rust
pub enum ReviewVerdict {
    Pass { rationale: String },
    Fail { reasons: Vec<ReviewIssue>, must_address: Vec<StageId> },
    PassWithFollowup { rationale: String, followups: Vec<Followup> },
}

pub struct ReviewIssue {
    severity: Severity,        // Blocker | Major | Minor
    section: HandoverSection,  // which part of the contract failed
    quote: String,             // the specific line the reviewer is challenging
    suggested_fix: String,
}
```

`Fail` is only legal if at least one issue has `Blocker`.
`PassWithFollowup` is the escape valve for "this is fine to merge, but
note these for the next stage" — without it, reviewers will reject
fine work because they have nowhere else to put the note.

### P3. Retry policy is in the runtime, not the model

```
work-session → peer-review
  Pass                 → advance to next stage / job
  PassWithFollowup     → advance, attach followups to next session's prompt
  Fail                 → redo-session (max 1) → peer-review
    Pass               → advance
    PassWithFollowup   → advance with followups
    Fail               → escalate (human, or a different-runner session)
```

Loop bound is fixed: **one** redo, period. Why one and not two: each
extra redo doubles cost while the marginal catch-rate drops fast (if
the worker got it wrong twice, a third try usually rationalises rather
than fixes). One redo forces the failure to become visible quickly;
more redos hide it under spend.

A second `Fail` does not retry again — it surfaces an
`Event::ReviewEscalated` and parks the job in `AwaitingHumanReview`.

**Reviewer errors are not `Fail`.** A timeout, malformed verdict, or
crashed reviewer process counts as `ReviewErrored`, not `ReviewFailed`
— it does not consume the redo budget. The runtime retries the
*reviewer* up to a small fixed cap (e.g. 2) before escalating; a
broken reviewer should not eat the worker's one allowed redo.

The retry count lives on the stage row (`stages.review_attempts`),
not in the model's prompt — the runtime enforces it. The redo session
sees a new handover-shaped artefact: the original handover + the
reviewer's `Fail` verdict + the must-address list.

### P4. Handover-for-redo is its own shape

A redo session's input is not just "the prior handover" — it is a
three-part packet:

1. The original handover from the failed work-session.
2. The reviewer's structured verdict (issues, quotes, suggestions).
3. A constraint section: "you have one attempt; address these
   specific blockers; do not re-architect."

Schema-wise this is a new wire type, not a `Handover`. Call it
`RedoBriefing` so the model and the runtime can tell them apart. It
lives in `codeless-types` next to `Handover` and `ReviewVerdict` — all
three are wire types crossing the runtime / UI / RPC boundary.

### P5. Reviewer selection is a policy hook

**The leverage move here: a different runner reviews the work, so a
single-model failure mode cannot pass itself.** A claude worker that
hallucinated a function name will hallucinate the same name when
asked to review its own diff; a codex reviewer reading the same diff
fresh will not. This is the strongest single argument for the gate.

Roll-out:

- MVP: same runner, fresh session. Validates the gate mechanics.
- Phase-2: different runner (claude work-session → codex peer-review).
  This is where the failure-mode-isolation actually kicks in.
- Phase-3: deterministic reviewer for cheap checks (lint / typecheck
  / test run as a "reviewer" with its own typed verdict) before the
  LLM reviewer ever runs. Cheap checks first amortise the LLM
  reviewer's token cost (see P8 budget discussion).

The interface stays `Reviewer -> ReviewVerdict`; what produces the
verdict is policy.

### P6. The review is an event-stream artefact

Every review writes:

- `runs/<job_id>/<stage_id>/review-<n>.md` (n = attempt number)
- `Event::ReviewStarted { review_id, stage_id, reviewer_kind }`
- `Event::ReviewDecided { review_id, verdict_kind, issue_count }`

The UI's Timeline interleaves these with the existing stage events.
A user reading the job page sees: stage started → stage completed →
review started → review failed → redo started → redo completed →
review started → review passed → next stage.

### P7. The review can challenge the handover itself

A common reviewer failure mode if you only show it the diff: it
reviews the code but never notices the handover claims something that
did not happen. Make the handover a first-class input: the reviewer's
prompt explicitly asks "does the handover's `done` list match the
diff? does `what you need to know` capture the real gotchas?" An
inaccurate handover is itself a `Fail` reason (`section:
HandoverSection::Done`).

Mechanical pre-check (paired with H2): before the LLM reviewer ever
runs, the runtime validates that every commit / file referenced in
the handover's structured fields actually appears in `git diff
<commit>^..<commit>`. A reference to a file the diff does not touch
is an automatic `Fail` with `severity: Blocker`, no LLM tokens spent.
The LLM reviewer is for judgement calls, not for catching
falsifiable claims the runtime can verify itself.

### P8. Cost / time caps on the review loop

A review session has its own budget envelope. The default should be
small — a review that takes longer than the work it reviewed is a
smell. The retry loop has a job-level cap too: review spend is
capped at `max(absolute_floor, N% of work_spend)`. The floor matters
because early stages have near-zero work-spend, so a pure percentage
cap escalates every early review. Suggested starting values:
`absolute_floor = 4k tokens`, `N = 50%`. Once exceeded, escalate
without running another review.

Acknowledge the per-stage cost: a separate-session reviewer (P1)
roughly doubles per-stage tokens. That is the price of the gate.
P5 phase-3 (deterministic reviewers run first) exists partly to
amortise this — a stage that fails `cargo check` never reaches the
LLM reviewer.

### P9. The review verdict feeds the next handover's prompt prefix

When the next stage starts, its prefix is not just the prior
handover; it is `handover + last_review_verdict`. The next stage sees
both "what the prior session claims it did" and "what the reviewer
agreed was actually done." When those disagree, the next session has
the signal to investigate before building on top.

### P10. Human override is always a verdict

A human reading the job page can click `Override -> Pass` or
`Override -> Fail` and the runtime treats it as a `ReviewVerdict` with
`reviewer_kind: Human`. Same wire shape, same event. The point is to
not have two parallel state machines (one for AI review, one for
human review) — there is one gate, multiple kinds of reviewer.

Human verdicts **reset** `review_attempts` to zero — they do not
consume the AI redo budget. The budget exists to bound AI loops, not
human ones; a human Pass that says "yes this is fine" should leave
the next stage with a full budget if the AI gate triggers again
later.

## What this implies for SCOPE.md

If we adopt any meaningful subset of the above, four things need to
move:

1. `stages` table grows: `review_attempts INT`,
   `last_review_verdict JSON`, `archived` stays as-is.
2. `Handover` wire type gets `schema_version`, `decisions`,
   `invariants`, `followups`, `artefacts` — and a `validate()` method
   the runtime calls before writing.
3. A new `Reviewer` trait alongside the `Runner` trait, with the same
   "runner factory" shape (a `ReviewerFactory` that produces a
   `Reviewer` per stage based on policy).
4. REVIEW stages in the template runner become real blocking gates,
   not advisory events. The `Event::ReviewRequested` emission stays;
   what changes is the runtime waits on the resulting verdict before
   advancing.

R1 (crate dependency direction) is unaffected. Crate ownership:
`Reviewer` *trait* and the `ReviewVerdict` / `RedoBriefing` wire
types live in `codeless-types` (mobile-safe, no process spawn).
`Reviewer` *implementations* live in `codeless-runtime` (pure-API
reviewers like Anthropic-direct) and `codeless-adapters-host`
(process-spawning reviewers like a CLI claude review wrapper). The
mobile shell can read verdicts and render them; it cannot host a
reviewer that spawns. R2 (single transport) is unaffected — the
verdict is a wire type, the UI consumes it via `RpcClient`. R4
(SQLite source of truth) gets reinforced: the verdict lands in the
`reviews` table, not in a sidecar JSON.

### Concurrency and idempotency

Two hazards the proposal does not address by default:

1. **Concurrent jobs in the same workspace.** When job A in repo X
   finishes and waits on a reviewer, job B in repo Y can finish and
   request a reviewer too. The review queue must be its own resource
   with bounded parallelism; otherwise a slow reviewer in one repo
   stalls every other repo's gate. Treat reviewers as a worker pool
   alongside the runner pool, sized independently.
2. **Reviewer crash mid-verdict.** If the runtime restarts after the
   reviewer started but before the verdict is committed, the runtime
   must distinguish "no verdict yet" from "verdict was Fail". The
   `reviews` row is created with `status: InFlight` *before* the
   reviewer is invoked; the verdict update is the commit point. A
   restart finding `InFlight` rows treats them as `ReviewErrored`
   (per P3's reviewer-error rule) and re-runs the reviewer, without
   consuming the worker's redo budget.

## Reconsider: rules + one audit agent vs. a framework

Before committing to typed verdicts, schema migrations, reviewer
factories, retry state machines, and crash-recovery for in-flight
review rows — ask whether a much smaller surface gets 80% of the
value:

**The two-part alternative:**

1. **Tighter SCOPE / JOB-MODEL / AGENT rules the worker follows.** A
   handover spec that's strict and exemplified is worth more than a
   `validate()` method on a wire type — the model is going to read
   the spec either way, and if the spec is good the validation
   catches almost nothing. Same for "ack the prior handover before
   coding" and "verify your `done` against `git diff` before writing
   the handover" — these are prompt rules, not runtime features.

2. **One job-audit agent, triggered after every stage.** A single
   subagent prompted to: read the handover, run `git diff`, decide
   `PASS` or `FAIL: <reason>` on a single line. The runtime parses
   that one line. No `ReviewVerdict` enum, no `ReviewIssue` struct,
   no `Severity`, no `must_address`, no per-section taxonomy. If the
   audit agent's prompt evolves, no schema migrates with it. If a new
   class of issue matters, you add a sentence to the prompt — not a
   variant to an enum that ships through three crates.

**What you give up by going prompt-first:**

- The UI cannot render structured verdicts as cards / charts / filters
  — it renders a transcript. For MVP this is fine; rich review UI is
  a Phase-2 problem the framework approach was solving prematurely.
- Queries like "show me every Blocker reason this week" become
  grep-the-transcripts instead of SQL. Real cost, but defer it until
  you have enough volume to care.
- Cross-tool consistency (claude reviewer, codex reviewer, lint
  reviewer all emit the same shape) is harder. Defer until P5
  phase-2 actually ships.

**What you keep with prompt-first:**

- The gate itself (PASS/FAIL blocks advancement).
- Bounded redo (P3 — count attempts in a SQLite int, no schema for
  the verdict shape needed).
- Different-runner reviewer (P5 — agent kind is a config string,
  not a trait).
- Audit transcripts as event-stream artefacts (P6 — write a markdown
  file, emit one event with the path).

**Recommended posture:** ship the prompt-first version first. Promote
to typed verdicts only when a *specific* downstream feature (a UI
view, a metrics dashboard, a policy engine) demands the structure.
Until then, every typed field is YAGNI on the framework side and
overhead on the agent side.

This collapses the implementation: most of the H ideas become bullet
points in `JOB-MODEL.md`; most of the P ideas become paragraphs in
the audit agent's prompt template. The runtime change is roughly: a
hook after each stage, a process invocation, a regex on the output,
a SQLite int for retry count. That is days of work, not weeks.

## Quick wins, in order

Built bottom-up so each tier validates the next. Stop at any tier
once it stops paying back.

### Tier 0 — docs only, ship today (hours)

1. **Tighten the handover spec in `JOB-MODEL.md`.** Worked example
   per section, an explicit anti-example, and the rule "non-empty
   `done` and `next` unless the stage aborted or terminated the
   plan". No code changes — the worker just gets a better spec.
2. **Add a JOB-LOOP rule: ack-then-code.** A stage that received a
   prefixed handover must restate the prior `next` in its own words
   in the first reply. Pure prompt rule.
3. **Add a JOB-LOOP rule: verify-before-handover.** Before writing
   the handover, run `git diff <stage-base>..HEAD` and confirm every
   path mentioned in `done` appears in the diff. Catches the most
   common reviewer-fail-reason without any reviewer.

### Tier 1 — minimum runtime hooks (1-2 days)

4. **Per-stage handover (H1).** Additive change in `TemplateRunner`;
   no schema migration. Pays off the moment any multi-stage job runs.
5. **Keyed handover discovery (H3).** Delete `find_latest_handover`,
   look up `(job_id, stage_id - 1)` directly. Removes the concurrent-
   job race. Code-only.
6. **Write-time handover validation (H7, light).** `Handover::parse`
   returns `Err` for blank `done` / blank `next`. No schema version,
   no structured fields yet — just the not-blank checks.

### Tier 2 — the audit agent (2-3 days)

7. **Wire one job-audit subagent after every stage.** Inputs:
   handover + `git diff` + stage title. Output contract: a single
   line, `PASS` or `FAIL: <one-sentence reason>`. Runtime parses the
   line and gates the next stage. No typed verdict, no retry — `FAIL`
   parks the job in `AwaitingHumanReview`.
8. **Persist the audit transcript** at
   `runs/<job_id>/<stage_id>/audit.md` and emit one event
   (`Event::AuditDecided { stage_id, outcome, transcript_path }`).
   Timeline-renderable; no schema for the verdict.

### Tier 3 — once the audit agent has shown value (1-2 days each)

9. **One bounded redo on FAIL** (P3, minimal). SQLite int
   `stages.audit_attempts`. On `FAIL`, re-queue the stage once with
   the audit transcript prepended. Second `FAIL` escalates.
10. **Different-runner audit agent** (P5 phase-2). Config string:
    "claude work, codex audit". The big leverage move — but only
    after Tier 2 has caught real issues and you trust the gate.
11. **Mechanical pre-check** (P7 hardening). Before invoking the
    audit agent, runtime verifies every path in handover's `done`
    appears in the diff; auto-`FAIL` with no tokens spent.

### Tier 4 — only if a specific downstream feature demands it

Promote to structured verdicts (P2), structured handover fields (H2),
schema versioning (H6), `ReviewVerdict` wire type, etc. Each of these
should be justified by a named UI view or metric, not by "it would
be nicer". Until then they are framework on speculation.

## Open questions

- Does the reviewer see the *prompts* the worker saw, or only the
  *outputs*? Seeing prompts catches "the worker ignored a constraint";
  not seeing prompts keeps the review honest about the artefact.
  Probably: see both, in separate clearly-labelled sections.
- When a redo runs, does it inherit the worker's claude session id
  (cheap, continuity) or start fresh (clean slate, no rationalising)?
  Probably: fresh — the point of a redo is to not double down.

(Two earlier open questions resolved in-line: the
`PassWithFollowup` blocking distinction is captured by P2's
`must_address: Vec<StageId>` field; the human-override budget rule is
in P10.)
