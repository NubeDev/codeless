# LOOP-CODER — the long-running coder loop, codeless-style

> **Sister docs.** This is the loop's *design intent and product
> constraint* doc. [`JOB-LOOP.md`](./JOB-LOOP.md) is the operational
> spec each tick follows (status file format, batching, scheduling,
> guardrails). [`SCOPE.md`](./SCOPE.md) is the product scope this
> loop sits inside (Repo → Job → Stage → Task → Review,
> single-tenant trust boundary, SQLite source of truth).
> If anything below contradicts SCOPE.md, **SCOPE.md wins** — fix
> this file rather than diverge.

## The one constraint that drives this doc

**A developer must be able to start a coder loop and walk away for
hours.** They wake up to a branch with shippable commits, a clear
handover file showing what landed and what halted, and a small
number of `REVIEW` stages waiting at the points where the loop
decided it needed a human.

24-hour operation is the design centre for codeless, in a way it
isn't for the agent frameworks codeless borrows from. Cursor's
background agents, Devin, Claude Code's `--continue`, openai/codex,
and rubix's own coder loop all touch parts of this problem; none of
them is built end-to-end around the "wake up to shippable commits"
contract. That contract is what this doc is for: "agent that codes
*for as long as it takes*, without melting under its own context,
without burning the budget, without silently clobbering work, and
without needing a human to babysit between ticks".

Everything else in this doc serves that constraint.

## What blocks this today

Before any of the design below is real, two primitives have to exist.
Both are unsolved as of this writing; the rest of the doc is
aspirational until they are.

**The scheduler.** A 24-hour unsupervised run needs *something* to
fire the next tick after the current one exits. `CronCreate` has
proved unreliable across harnesses (durable flag ignored, one-shot
crons queued but never fired a fresh session — see
`DOCS/sessions/2026-05-12-ux-grind.md`). `ScheduleWakeup` and the
`/loop` skill are agent-side primitives with the same harness
dependency. The honest answer is that **codeless-runtime owns the
cron**: it's already a long-lived service, scheduling is in its
problem domain, and a real cron poking the agent runner sidesteps
every harness quirk. The agent loop does not self-schedule; it just
exits and trusts codeless-runtime to invoke the next tick. Until
this lands, every claim in this doc about "wake up to commits" is
aspirational and every tick still needs a human to say "next".

**The runner-side fresh-session rule.** Fresh-session-per-tick at
the *agent* layer is meaningless if the *runner* it invokes carries
session state forward. The Claude Code CLI exposes `--resume <id>`
and codeless's `ClaudeRunner` forwards a `resume_id` to it
([ai-runner/src/runners/claude.rs:164](../ai-runner/src/runners/claude.rs#L164)).
If a tick passes the prior tick's `resume_id`, you've recreated the
long-lived orchestrator inside the CLI subprocess — context grows
again, one layer down. **The rule: the coder loop never passes
`resume_id` (or any equivalent session token) between ticks. Each
tick is a fresh agent invocation *and* a fresh CLI/REST
invocation.** The cost of re-onboarding the CLI onto the codebase
every tick is real but bounded; the cost of unbounded context bloat
in the CLI is not. The job-memory RAG (see § Rig) is what makes
re-onboarding cheap enough to live with.

## Why "fresh session per tick" is load-bearing

Every coding agent eventually runs into the same wall: **context grows
faster than usefulness**. An LLM session that lasts an hour of real
work has accumulated tool-call results, file dumps, intermediate
reasoning, and dead ends. By hour two it's slower, more expensive,
and worse at the task than a fresh session pointed at the current
state of the repo would be. Larger context windows (1M+) don't fix
this — they just delay the wall and inflate the per-token cost of
hitting it.

Beyond raw window pressure, "fresh" buys two more things that matter
for unsupervised runs: **cache invalidation of stale beliefs** (the
agent that did stages 1–4 confidently "knows" things about the repo
that may be wrong by stage 5, and a fresh agent re-derives them from
the current code), and **per-tick cost predictability** (a fresh
prompt is a known-shape input — you can reason about p95 token usage
per tick instead of an unbounded curve).

This is bounded: fresh-per-tick beats long-lived **for unsupervised
multi-hour runs**. For a 30-minute interactive session the long-lived
orchestrator is cheaper and faster — you skip N re-reads of the status
file and N cold-starts on the codebase. Don't apply this model to
short supervised sessions; that's not what it's for.

The rubix coder loop (see [§ Comparison](#comparison-with-the-rubix-coder-loop))
runs one long-lived in-process orchestrator with a budget enforcer.
That works for sessions measured in hours. It cannot work for
sessions measured in days — the orchestrator's own conversational
context becomes the bottleneck long before any cost ceiling trips.

**Codeless solves this by treating each session as disposable.**
A session wakes up, reads the handover from disk, picks the next
stage, does the work, commits and pushes, writes the next handover,
exits. The next session is a *fresh* agent — no in-memory state, no
prior conversation, no accumulated tool output. It rebuilds its
understanding from:

- `runs/<job>/handover.md` — what the previous session knew that the
  next one needs (Done / Next / What you need to know / Don't redo
  / Where I stopped). The contract between sessions.
- `runs/<job>/log.md` — append-only audit; mostly for the user, the
  next session usually doesn't need it.
- `.codeless/jobs/<name>.yaml` — the job's stage list. Re-read every
  session so user edits take effect.
- `CLAUDE.md` / `CODELESS.md` — durable per-repo rules and memory.
- The git history on the remote — what the previous session
  actually changed.
- The codebase itself.

See [`JOB-MODEL.md`](./JOB-MODEL.md) for the file formats and
[`JOB-EXAMPLE.md`](./JOB-EXAMPLE.md) for a worked run.

That's it. Everything else is disposable. The agent that did
stages 1–4 has no privileged view of stages 5–9 that a fresh agent
starting at stage 5 wouldn't have — *provided* the runner it invokes
is also fresh (see "What blocks this today" above; passing
`resume_id` to the CLI silently breaks this property).

This design choice has three direct consequences:

1. **Context never grows unbounded.** Each session starts with an
   empty conversation. Eight hours of work is six-to-eight fresh
   sessions, not one accumulating monster.
2. **The handover is the contract, not a convenience.** If a session
   writes a handover the next session can't act on, the loop is
   broken. If a session *doesn't* write something the next session
   needs, the loop is broken. The five fixed sections in
   `handover.md` are load-bearing.
3. **The push step is non-negotiable.** A session that commits but
   doesn't push leaves the next session reading a stale remote. The
   next session will plan against state it can see and risk
   clobbering work it can't.

These three are the spine of [`JOB-LOOP.md`](./JOB-LOOP.md). Read
that doc for the operational rules; this doc explains *why* those
rules exist.

## What unsupervised 24-hour operation actually requires

Listing them so future contributors know which features earn their
keep against this constraint and which are nice-to-haves.

### Required (a long unsupervised run is impossible without these)

- **Fresh session per session boundary.** Above.
- **Durable handover file + remote git as the only carriers between
  sessions.** Anything in-memory is gone by morning. The handover
  format is in [`JOB-MODEL.md`](./JOB-MODEL.md).
- **Idempotent commit + push.** A session that crashes between
  commit and push must be re-runnable without duplicating work or
  conflicting. `git commit` with no diff is a no-op; `git push`
  with no new commits is a no-op.
- **Crash-resumption invariant.** Each stage produces two commits
  in order: the stage commit (code only) and the handover commit
  (`handover.md` + `log.md`). Both are pushed before the next
  stage starts. Three crash points, three recoveries:
  - **After stage commit + push, before handover commit:** the
    stage is fully landed; the handover is stale. The next session
    reconciles Done/Next by diffing the handover against `git
    log`. The handover commit is idempotent — replays of a fully
    handed-over stage produce no diff.
  - **After handover commit, before push:** the next session's
    first action is push. Idempotent.
  - **Mid-stage, dirty worktree:** the next session discards the
    dirty diff (no commit), the stage replays from scratch.
- **Push after every stage, never deferred.** Not "at the end of
  the session". Every stage. Including the handover commit at the
  end of a session — which is itself a stage of sorts.
- **Codeless-runtime owns scheduling, not the agent.** The agent
  exits cleanly; codeless-runtime fires the next session. An agent
  that silently exits without writing the handover is a bug, but it
  is not also responsible for scheduling its successor. See § What
  blocks this today.
- **Per-job worktree.** A crashed or runaway session can't damage
  the user's main checkout. The user can inspect what a halted
  session left behind by opening the worktree in any editor.
- **Each session is bound by `CLAUDE.md` like any other contributor.**
  When the loop spawns an agent that edits codeless's own codebase
  (or any repo with rules in `CLAUDE.md`), those rules apply
  unchanged. The loop has no exemption from the crate dependency-
  direction rule, the `RpcClient`-only-import rule, the no
  `Foo.web.tsx`/`Foo.mobile.tsx` rule, or the no-comment-status
  rule. Trip any of them → session halts, marks the stage failed,
  writes the reason into `handover.md`.

### Required for *useful* long-run operation

- **Budget ceilings on the loop itself.** Not just per-job cost
  caps — the loop needs a soft (`Escalate`) and hard (`Halt`)
  ceiling on cumulative cost, wall time, and token usage across the
  whole job. Without this, one runaway stage burns the night's
  budget on a stuck `claude` instance and the operator wakes up to
  "out of credit, no progress".
- **REVIEW stage support.** A stage whose title starts with
  `REVIEW` halts the loop and waits for a human. This is different
  from a halt-on-failure — REVIEW is "I did the work cleanly, I'm
  not going to publish without you". See JOB-MODEL.md.
- **Coherent halt-on-failure.** A failed verify stops the loop, full
  stop. No retry-on-failure, no "skip this stage and try the next" —
  silent skips are how a long run produces six green stages and
  three quietly broken ones. The failure goes into the handover so
  the user can see it on wake-up.
- **Job-memory RAG over prior runs.** Fresh-session-per-tick has one
  real weakness: a fresh agent doesn't know that a previous session
  (or previous job) already tried approach X and it failed for
  reason Y. The handover's `Don't redo` section carries this within
  one job; a Rig-backed RAG over prior `runs/*/log.md` and
  `CODELESS.md` notes carries it across jobs. Helper role per
  SCOPE.md, not in the coding step itself.

### Nice but not load-bearing

- A pretty UI showing what each session did. The handover + log +
  git log cover the audit trail; the UI is a render.
- Real-time notification on halt. Convenient, not load-bearing — the
  handover is the source of truth.
- Multi-repo jobs (one job touching two repos). Today the loop is
  one job per (repo, branch); fan-out across repos is a future
  feature, not a long-run requirement.

## Comparison with the rubix coder loop

Rubix ships `crates/domain-coder` (in `rubix-agent`) — same JOB-LOOP
contract codeless follows, implemented as a long-lived in-process
orchestrator with a `StepDriver` trait and a `BudgetEnforcer`. See
`rubix-agent/docs/sessions/ai/CODER-LOOP.md` for the spec and
`rubix-agent/crates/domain-coder/src/` for the implementation.

Both loops share the same DNA: markdown as the persistence layer,
plan → implement → verify → record → commit/push ordering, and the
same per-stage discipline. The differences are about *how many
processes* hold the loop together over time.

| Dimension | Rubix coder loop | Codeless coder loop |
|---|---|---|
| Session model | One long-lived in-process orchestrator. | Fresh agent per session. |
| Bounded session length | Limited by orchestrator context + budget. | Effectively unbounded — context resets every session. |
| Session boundary | A `LoopDriver::run_tick` call. | A whole agent process invocation, fired by codeless-runtime's scheduler.[^sched] |
| Source of truth between sessions | Markdown status doc (read once at session start, written in-process). | `handover.md` + `log.md` on disk, pushed to remote. Every session re-reads. |
| Crash recovery | Re-reads the doc from disk via `MarkdownStatusDoc::from_file` — survives a crash. What it can't survive is its own context bloat over hours. | Always re-reads `handover.md` + remote git. The real difference isn't on-disk vs in-memory state — both loops use markdown — it's one process running N stages vs N processes running 1 stage each. |
| Concurrency | One session at a time, locked on `(repo, branch)`. | Many concurrent jobs across many repos (SCOPE.md). |
| Budget enforcement | `BudgetEnforcer` with soft + hard ceilings. | Per-job cost cap; loop-level ceilings are an open item (see § Borrowed ideas). |
| Approval gate | `[?]` state in the status doc. | A stage title prefixed `REVIEW` halts the loop. Mirrored as a `Review` row in SQLite for the UI. |
| Step driver | `StepDriver` trait, injected. | Inline shell-outs to git inside each session. |

[^sched]: codeless-runtime's session scheduler is the unsolved load-bearing primitive flagged in § "What blocks this today". The comparison row describes the *intended* model, not what runs today.

### Where rubix is stronger

1. **Executable loop.** The loop rules are a Rust crate
   (`MarkdownStatusDoc` parser, `BatchPlanner`, `LoopDriver`),
   unit-testable, deterministic without an LLM in the interpretation
   path. Codeless's loop is prose in `JOB-LOOP.md` that an LLM has to
   re-interpret every tick.
2. **`StepDriver` trait.** Side effects (cargo, git, mani) are
   injected, so orchestration is testable and a new driver (e.g. for
   a non-git VCS) is a new file rather than a rewrite.
3. **Named crash-resumption invariant.** Rubix explicitly names
   `update_status` as the commit point; codeless's JOB-LOOP describes
   the order without naming the invariant.
4. **Budget ceilings.** Two-tier (escalate, halt) on tokens, cost,
   and wall time. Codeless's caps are per-job, not per-session.
5. **`verify.sh` contract.** Typed cwd, env allowlist, timeout,
   signals, stdout truncation, discovery order. Codeless's verify
   today is implicit (`cargo test && cargo clippy && cargo fmt &&
   pnpm tsc`) and only works because codeless drives its own repo.
6. **Explicit review-gate semantics.** Rubix's `[?]` state in the
   status doc is the same idea as codeless's `REVIEW`-prefixed
   stage: a checkpoint where the loop intentionally halts. Rubix
   has it nailed down as a state; codeless treats it as a string
   prefix today. Worth tightening into a real state.

### Where codeless is stronger

1. **The 24-hour run is the design centre.** Rubix's loop assumes
   the orchestrator process stays alive across the whole session;
   codeless's loop assumes it does not. The latter is the only way
   to run for days without context collapse.
2. **Multi-job, multi-repo concurrency from day one.** Rubix's
   coder loop is one-session-at-a-time; codeless is many-jobs-many-repos
   with a scheduler and per-repo caps.
3. **SQLite as source of truth for *runs*.** Rubix's status doc is
   the only persistent state. Codeless persists every run, every
   event, every review to SQLite; the status file is for the loop's
   self-driving, the database is for the product surface.
4. **Typed wire event stream.** SSE + specta-generated TS types mean
   every UI (and every external script via the future MCP surface)
   sees the same event vocabulary in real time. Rubix's coder loop
   has a status doc, not an event stream.
5. **Single UI for four shells.** Browser, Tauri desktop, iOS,
   Android, all driven by the same `RpcClient`. Rubix's coder loop
   doesn't address shells.

### Ideas we should borrow

In priority order, highest payoff first. In a 24-hour-loop world the
cost of *not* borrowing dominates the cost of borrowing, so ordering
by payoff (not by implementation cost) is the right call.

1. **Soft + hard budget ceilings on the *loop*, not just the
   job.** The single biggest gap relative to the constraint. A
   long unsupervised run without an aggregate ceiling is exactly
   the "wake up to no credit, no progress" failure this doc warns
   about; per-job caps don't help if 20 jobs each spend up to their
   cap. Reuse the existing `cost_cap_cents` / `wall_clock_cap_ms`
   shape; add an `escalate_at` ratio and a per-loop accumulator.
   Three verdicts: `Continue`, `Escalate`, `Halt`. The runtime
   checks the verdict before firing each session. The handover gives
   *visibility* on what happened overnight; the budget enforcer
   prevents the bad outcome in the first place.
2. **Name the crash-resumption invariant.** One paragraph in
   JOB-MODEL.md: "the handover write lands in the same commit as
   the code change; push follows; push is idempotent, so a crash
   after the handover commit replays only push." Stops a future
   contributor reordering the steps. Almost free to land.
3. **Named `LoopStep` enum.** Rubix encodes plan → implement →
   verify → record → commit/push as an enum
   ([driver.rs:20](../../rubix-workspace/rubix-agent/crates/domain-coder/src/driver.rs#L20))
   instead of prose. This is the mechanism that turns the
   crash-resumption invariant from a paragraph into a unit test.
   Lands alongside (or inside) the crate extraction below.
4. **Round-trip-preserving handover parser.** Rubix's
   `MarkdownStatusDoc::parse`/`render` preserves preamble and
   trailer verbatim
   ([status_doc.rs:80-89](../../rubix-workspace/rubix-agent/crates/domain-coder/src/status_doc.rs#L80-L89)).
   This is **a prerequisite for a contract JOB-MODEL already
   makes**, not a future enhancement: JOB-MODEL promises the user
   can edit `handover.md` between sessions to inject knowledge or
   rewrite the `Next` list. An agent that re-renders the handover
   and silently drops the user's edits breaks that promise on the
   next session. Until a round-trip-preserving parser lands, the
   agent must read the handover, only mutate the sections it owns
   (Done / Next / What you need to know / Don't redo / Where I
   stopped), and never re-serialise the entire file. Copy rubix's
   behaviour when extracting the crate.
5. **Port the framework into a `codeless-loop` crate.** Handover
   parser, stage picker, `StepDriver` trait — transport-agnostic,
   unit-testable. Codeless then ships a real `/rpc/loop_*` surface
   and the LLM stops being the interpretation layer for "what
   stage is next" and "is this handover well-formed". The LLM
   re-interpreting prose rules every session is a real cost in an
   8-hour run. A parser + planner crate makes interpretation
   deterministic and free. Largest change; largest payoff. Defer
   until at least one more consumer exists so the trait shape is
   driven by two callers. **This does not violate the
   fresh-session-per-session rule** — the crate is the *parser and
   planner*, not the orchestrator. Each session still constructs
   it fresh, runs one stage or batch, exits.
6. **`verify.sh` contract** if and when codeless drives a user's
   repo with a complex verify story. Today one shell command in
   `config.yaml` is enough.

What we deliberately do *not* borrow:

- The long-lived in-process orchestrator. That's the very thing the
  fresh-session-per-tick model rejects.
- The "profile vs framework" split (`domain-coder` as a profile of a
  generic `domain-job-loop`). Codeless's product is the coder loop;
  there is no second profile to factor against.
- MCP `coder.*` aliases as a separate surface. Codeless's MCP
  surface (SCOPE.md "MCP surface") already mirrors the RPC trait;
  loop control lives on the same RPC trait, no aliases needed.
- Skill packaging (`com.<org>.coder.<task>` extension dirs).
  Codeless's answer to "I want to add a new kind of job" is **YAML
  /TOML job templates** per SCOPE.md "Coding loop" — declarative,
  versioned in the repo, no extension-dir lifecycle. That's the
  right shape for the product.

### Rig is not in the coder loop

Per SCOPE.md "Helper role — Rig, optional, never gates a job", Rig
is the LLM-API client for *helper* roles (planner, reviewer,
summariser, job-memory RAG, cheap-model routing). It is **never**
inside a session's coding step. A session may invoke Rig-backed
helpers — a summariser to generate the commit message, a reviewer
to attach a human-readable summary to a `REVIEW`-stage `Review`
row, a RAG lookup against prior `runs/*/log.md` so a fresh session
can see "we tried this approach and it failed for reason Y" without
prompt bloat — but the `Runner` that does the coding is always a
CLI wrapper (Claude Code, Codex, Copilot) or a direct API runner
(Anthropic, OpenAI, OpenAI-compat). This rule closes the door on
the obvious future drift of "let's have Rig inject context into the
coding session"; it would create two parallel paths to the same
model with different auth, cost accounting, and cancellation
semantics.

Job-memory RAG specifically is the highest-payoff Rig use against
the long-run constraint, because it directly attacks the main
weakness of fresh-session-per-session (no memory of prior dead ends
across jobs — within one job the handover's `Don't redo` section
covers it). See SCOPE.md "Where Rig is genuinely useful long-term".

## Open items

The two load-bearing blockers (scheduler, runner-side `resume_id`
rule) are called out in "What blocks this today" at the top. What
remains:

- **Loop-level budget.** Today each job has its own cap; an
  unsupervised loop running 30 stages over 24 hours has no aggregate
  ceiling. Borrow rubix's two-tier `Continue` / `Escalate` / `Halt`
  model. Highest payoff of the remaining items.
- **Review-gate state.** Today a stage title prefixed `REVIEW`
  halts the loop. Cheap and human-readable but fragile to typos.
  Decide whether to tighten this into an explicit state in the
  YAML (e.g. `kind: review` on the stage) or keep the prefix
  convention with a validator that catches near-misses
  (`REVEIW:` etc.) at queue time.
- **Codeless-runtime scheduler design.** The recommendation in
  "What blocks this today" is that codeless-runtime owns the cron
  and invokes sessions directly. The detailed design — process
  model, how the runtime picks up a halted vs in-progress job,
  how it caps concurrent sessions across jobs — is still TBD.
