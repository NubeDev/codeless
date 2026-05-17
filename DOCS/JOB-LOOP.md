# Job Loop — Staged Autonomous Build Workflow (Codeless)

A pattern for letting Claude Code chip away at a multi-stage Codeless build
on its own: implement a **logical batch of stages sized by complexity**,
**commit + push each stage**, update status, **schedule one fresh one-shot
tick ~1 min out, exit**. The status file and git carry all state across
ticks — the in-memory conversation is treated as disposable.

Codeless itself is the system being built. Use this loop to dogfood the
phases laid out in [SCOPE.md](./SCOPE.md): Phase 1 crate skeleton, Phase 2
runtime, Phase 3 browser MVP, etc. Each phase is many stages; this loop is
how you grind through them.

> **Batch, don't trickle.** A tick that finishes one trivial 5-line edit
> and then sleeps 1 minute is the failure mode this doc exists to prevent.
> Every stage in the status file carries a complexity tag (`S`/`M`/`L`);
> each tick groups as many related stages as fit the budget below. Sleeping
> between ticks is for context reset, not for pacing the work.

## Why

- Long builds outlive a single context window. Compaction is lossy; the
  status file is not.
- Each stage lands as its own commit *and is pushed*, so progress is durable
  even if the laptop dies mid-run, and a bad tick is a one-line revert.
- Wake-ups are cheap. The model can stop, sleep, and resume without a human
  in the loop, but a human can interrupt at any tick boundary.
- A self-chaining one-shot schedule means a halted/`DONE` tick stops the
  loop *automatically* — no leftover cron to clean up, no recurring job to
  fight a guardrail.

## Hard rules (read these first)

> ### ⛔ RULE 0 — YOU **MUST** SCHEDULE THE NEXT TICK. NO EXCEPTIONS.
>
> A tick that finishes a stage and **does not** schedule the next wake-up
> (or report DONE because all stages are `[x]`) is a **bug**. The whole
> point of JOB-LOOP is autonomous progress; a one-shot tick that leaves the
> work half-done defeats the design.
>
> **At the end of every tick you MUST do exactly one of these three things:**
>
> 1. Call `CronCreate` with `recurring: false` for a single fresh tick
>    ~1 minute from now — see
>    [§ Scheduling — exactly how to do it](#scheduling--exactly-how-to-do-it).
> 2. Report `DONE` because every stage in the status file is `[x]`.
> 3. Halt **and explain to the user, in plain text, exactly why you can't
>    schedule** — see [§ If you cannot schedule](#if-you-cannot-schedule).
>    Silently exiting is forbidden.
>
> Recurring schedules are **forbidden**. Each tick owns the responsibility
> of scheduling exactly one successor. If a tick halts on a guardrail, the
> loop stops — that is the design.
>
> Canonical reference for the scheduling tools:
> <https://code.claude.com/docs/en/scheduled-tasks>

1. **Code style:** every stage must follow the rules in
   [`CLAUDE.md`](../CLAUDE.md) at the codeless repo root. That file is the
   project-memory contract — crate dependency direction, no
   `@tauri-apps/api/core` in UI modules, no `Foo.web.tsx` / `Foo.mobile.tsx`,
   `RpcClient`-only imports, plus the comment-quality rules below. The loop
   is not an excuse to skip them. If `CLAUDE.md` doesn't exist yet, the
   first stage of Phase 1 creates it ([SCOPE.md](./SCOPE.md), Phase 1
   deliverables).
2. **Commit AND push every stage via mani.** The mani workspace lives at
   `~/code/rust/mani.yaml` (per [SCOPE.md](./SCOPE.md) "Multi-repo dev
   setup"). Never raw `git commit` / `git push`. The push step is
   **non-negotiable** and must happen **before the tick exits** — see
   [§ Push every tick — why it's non-negotiable](#push-every-tick--why-its-non-negotiable).
3. **One logical batch per tick, sized by complexity.** Each stage in the
   status file has a complexity tag — `S` (small, ≤ ~15 min, mechanical),
   `M` (medium, real thinking, one focused area), `L` (large, must be
   pre-split into S/M sub-stages before the loop touches it). A single
   tick may complete **any one** of:
   - up to **4 contiguous `S` stages** that share an area of the codebase,
   - **1 `M` stage** (optionally plus 1 closely-related `S`),
   - the next sub-stage of an `L` (treat the sub-stage as `S` or `M`).

   Each individual stage still gets its own verify + commit + push (so a
   bad stage is a one-line revert), but **all of them happen inside the
   same tick** before scheduling the successor. Stop the batch early if
   verification fails, the diff grows beyond what was planned, or context
   is getting heavy — schedule the next tick from wherever you stopped.
4. **The status file is the source of truth.** Update it in the same commit
   as the code change so a `git log -p DOCS/sessions/<file>` tells the
   whole story.
5. **Cross-shell and cross-platform rules apply.** Codeless is one Rust core
   + one React UI driving browser / desktop / iOS / Android (per
   [SCOPE.md](./SCOPE.md) Rule 1 + Rule 5). When a stage touches a UI
   module it must not import `@tauri-apps/api/core` directly. When a stage
   touches a Rust crate it must not violate the iOS-safe / Android-safe
   columns of the crate table. Both rules are enforceable by `cargo check`
   and CI grep — if either trips, mark the stage `[!]` and halt.

## Components

### 1. The status doc — single source of truth

Lives at `DOCS/sessions/YYYY-MM-DD-<slug>.md` inside this repo. Survives
`/compact`, `/clear`, fresh sessions, and crashes.

Naming:
- `YYYY-MM-DD` is the date the loop kicked off (not today's tick).
- `<slug>` is a short kebab-case description of the goal — e.g.
  `phase-1-crate-skeleton`, `phase-2-scheduler`, `rpc-trait-and-specta`.
- One file per loop run. If the same goal is resumed days later, keep
  the original filename so the git history stays linear.

Throughout this doc, "the status file" refers to **that** dated session
file. The path is recorded in the kickoff prompt so every tick opens the
same file.

```md
# Build status — Phase 1 crate skeleton

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: `CLAUDE.md` (repo root).
> 2. **One logical batch per tick.** Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3: up to 4 contiguous S in one
>    area, OR 1 M (+ optional related S), OR the next sub-stage of an L.
>    Verify + commit + push **each stage** via mani before moving to the
>    next stage in the batch.
> 3. **You MUST schedule the next tick before exiting** — call
>    `CronCreate` with `recurring: false` for a single fire ~1 min from
>    now. If all stages are `[x]`, report `DONE` instead. If you cannot
>    schedule, **do NOT exit silently** — tell the user which stage
>    finished, exactly why scheduling failed, and how to re-kick. See
>    JOB-LOOP.md "If you cannot schedule".
> 4. Update this file in the **same commit** as the code change.
> 5. ⛔ **COMMIT _AND_ PUSH BEFORE THE TICK ENDS.** Pushing is not
>    optional and not "later". A tick that ends with unpushed commits
>    means the next tick (or the next agent, after `/clear` or a fresh
>    session) sees stale remote state and can clobber or duplicate work.
>    `./bin/mani --config mani.yaml run commit --projects codeless` then `mani run push --projects
>    codeless` — both, every tick, no exceptions. If push fails, mark
>    the stage `[!]` and halt. Never `--force`, never `--no-verify`.
> 6. ⛔ **CODE COMMENTS ARE LOAD-BEARING — WRITE THEM CAREFULLY.**
>    Comments are how the *next* AI agent (and the next human) understands
>    intent. Rules:
>    - Explain **why**, not what. The code already says what.
>    - **No emojis.** Anywhere. Ever.
>    - **No task-status comments.** Never reference stages, ticks,
>      milestones, "added in stage 3", "TODO from M5", "fixed for ticket
>      X". Comments describe the code as it stands, not the task that
>      produced it.
>    - **Long-term framing.** Write for someone reading this in 6 months
>      with zero context — invariants, constraints, why this approach
>      over the obvious one.
>    - **Normal length.** A short line where one helps. A short paragraph
>      where the *why* is genuinely subtle. No multi-paragraph essays,
>      no decorative banners, no ASCII art.
> 7. ⛔ **CROSS-PLATFORM REACH IS ENFORCEABLE.** Stages that touch Rust
>    crates respect the iOS-safe / Android-safe columns in
>    `DOCS/SCOPE.md` "Crate layout". Stages that touch UI modules import
>    only `RpcClient` — never `@tauri-apps/api/core` directly. Trip
>    either rule → mark stage `[!]` and halt.

File: DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md
Goal: <one sentence — what "done" looks like for this loop run>
Started: 2026-05-12
Last tick: 2026-05-12 11:14
Current stage: 3 / 8

Repo:        codeless
Branch:      feat/phase-1-skeleton
Memory policy: compact every 3 stages
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag is mandatory.
`L` stages must be split into S/M sub-stages before being worked.

- [x] 1. [S] Cargo workspace + crate stubs
- [x] 2. [S] codeless-types: Repo/Job/Stage/Task/Event structs
- [ ] 3. [M] codeless-rpc trait + in-process impl       ← next
- [ ] 4. [S] specta wire-type generation (Rust → TS snapshot test)
- [ ] 5. [S] sqlx initial migration (per Appendix A schema)
- [ ] 6. [M] codeless-runtime state-machine skeleton + MockRunner
- [ ] 7. [S] tracing baseline (JSON to stdout)
- [ ] 8. [S] CLAUDE.md at repo root (per SCOPE.md Phase 1)

Likely batching (planning hint, not a contract):
- Tick A: stage 3 (M). Tick B: stages 4 + 5 (2×S, both schema-adjacent).
- Tick C: stage 6 (M). Tick D: stages 7 + 8 (2×S, mechanical).

## Notes
- Stage 2: ULID via `ulid` crate, serde via `serde_with::DisplayFromStr`.
- Stage 3 plan: trait first, in-process impl backed by tokio mpsc.

## Blockers
(none)
```

Rules:
- Exactly one stage is `← next` at any time. Move the marker forward as
  each stage in the batch lands.
- Every stage carries an `[S|M|L]` complexity tag. If a stage is missing
  one, halt — sizing is required for sane batching.
- A stage moves to `[x]` only after its commit **and push** land green.
- A failing stage becomes `[!]` with a one-line reason; the loop halts.
- An `L` stage must be split into S/M sub-stages **before** the tick
  begins implementation. Splitting can be a tick of its own (an `S`
  planning stage that only edits the status file).

### 2. The tick

Each wake-up runs this procedure end-to-end. No skipping steps.

1. **Pre-flight**
   - `git status` must be clean. Dirty tree → halt, write blocker, exit.
   - Read the status file. If unparseable or no `← next` → halt.
   - If all stages `[x]` → write "DONE" line, **do not reschedule**, exit.
2. **Decide the batch.** Look at the next `[ ]` stages and their
   complexity tags. Pick the largest group that fits the rule in
   "Hard rules" #3:
   - up to 4 contiguous `S` in one area, OR
   - 1 `M` (+ optional 1 closely-related `S`), OR
   - the next sub-stage of an `L`.

   If the next stage is `L` and not yet split, the batch is a single
   `S` planning stage that splits it in the status file and exits. Write
   the chosen batch in chat in one line ("batch: stages 4, 5") so a
   human watching can interrupt before any code lands.
3. **For each stage in the batch, in order, run steps 3a–3f below.**
   Stop the batch (and schedule a successor for whatever's left) the
   moment any stage halts, verification fails, or the diff visibly
   exceeds what was planned.

   3a. **Plan the stage** — read only the files this stage touches.
   Re-skim the relevant section of [SCOPE.md](./SCOPE.md) (and
   [`CLAUDE.md`](../CLAUDE.md) once it exists) so the change lands in
   the right crate and respects the layering rules.

   3b. **Implement** — minimum diff to satisfy the stage. No drive-by
   refactors, no hypothetical-future abstractions.

   3c. **Verify** — `cargo test --workspace` for Rust changes,
   `cargo clippy -D warnings` and `cargo fmt --check` always.
   `tsc --noEmit` and `vitest` for UI changes. specta snapshot test
   if the change touched a wire type. PTY / runner tests use a fake
   `claude`-style binary on an explicit `PATH` (per SCOPE.md Testing
   strategy) — never the developer's host install.
   - Failure → mark stage `[!]`, write reason under Blockers, halt
     (do not continue the batch, do not schedule).

   3d. **Update the session status file** (`DOCS/sessions/
   YYYY-MM-DD-<slug>.md`) — check the box, move `← next`, bump
   `Current stage`, set `Last tick` to the current local timestamp,
   append a Notes line if anything non-obvious was decided.

   3e. **Commit via mani** (one commit covers the code change + status
   file update). Note the mani env-var form: `KEY=value` is a positional
   argument *after* the task name, not a shell prefix:
   ```sh
   ./bin/mani --config mani.yaml run commit --projects codeless \
     MSG='stage N: <stage title>'
   ```
   3f. **Push via mani — required, every stage:**
   ```sh
   ./bin/mani --config mani.yaml run push --projects codeless
   ```
   If push fails (auth, non-fast-forward, hook reject) → mark stage `[!]`,
   write reason, halt. Do NOT re-attempt with `--force`.

   After 3f, loop back to 3a for the next stage in the batch (if any).
4. **Schedule next tick — REQUIRED.** Call `CronCreate` with
   `recurring: false` for a one-shot fire ~1 min from now. The `prompt`
   is the same `/loop` body verbatim (see kickoff template). If you
   cannot, follow [§ If you cannot schedule](#if-you-cannot-schedule) —
   do **not** exit silently. See
   [§ Scheduling — exactly how to do it](#scheduling--exactly-how-to-do-it).
5. **Exit.** No summary, no extra work — the next tick is a fresh start.

## Push every tick — why it's non-negotiable

> ⛔ **Every tick must end with the latest commit pushed to the remote.**
> No "I'll push at the end of the run." No "the work is in `git log`, that's
> good enough." Push, every tick, before exit.

Why this matters specifically for JOB-LOOP:

- **The next tick is a different agent.** Each tick is a fresh session
  fired by `CronCreate`; the agent reading the status file only sees
  what's on the remote. Unpushed local commits are invisible — that agent
  will plan against stale state and risk clobbering or duplicating work.
- **The laptop is not a database.** A loop can run for hours. Anything that
  isn't pushed disappears the moment the machine sleeps wrong, the session
  exits, or the disk has a bad day.
- **The status file only tells the truth if the commits backing it are
  reachable.** A status file that says stage 5 is `[x]` while stage 5's
  commit lives only in a local reflog is actively misleading.
- **Recovery depends on it.** "Loop is `[!]`, fix and re-kick" only works if
  re-kicking from the remote reproduces what the loop saw. Unpushed work
  breaks that.

**The rule, stated precisely**

At the end of every tick, before calling `CronCreate` / reporting `DONE`:

1. `./bin/mani --config mani.yaml run commit --projects codeless` — exits 0.
2. `./bin/mani --config mani.yaml run push --projects codeless` — exits 0.
3. `git status` clean, `git log @{upstream}..HEAD` empty (nothing ahead).

If any of those fail, the stage becomes `[!]`, you halt, and you tell the
user. **You do not schedule the next tick on top of an unpushed commit.**

## Scheduling — exactly how to do it

> Canonical Anthropic docs:
> <https://code.claude.com/docs/en/scheduled-tasks>

**One mechanism only: `CronCreate` with `recurring: false`, called as the
last action of every tick, for a single fresh fire ~1 minute from now.**

That's it. No recurring crons. No `ScheduleWakeup`. No Routines. Each tick
schedules exactly one successor and exits. If a tick halts on a guardrail,
no successor is scheduled, and the loop stops cleanly.

### How to compute the cron expression

`CronCreate` takes a 5-field cron in **local time**: `M H DoM Mon DoW`. For
a one-shot ~1 min from now, pin all four numeric fields and leave DoW as `*`.

1. Run `date "+%Y-%m-%d %H:%M:%S"` to get current local time.
2. Add 1 minute. Round to the next whole minute if needed.
3. **Pick an off-minute** if rounding lands you on `:00` or `:30` — the
   scheduler fires those up to 90 s early. Bump by 1 minute to dodge it.
4. Format as `M H DoM Mon *`. Months and days are not zero-padded in cron
   (`5` not `05`).

Example: it's `2026-05-12 11:12:25`. +1 min = `11:13`. Off-minute, no
adjustment needed. Cron: `13 11 12 5 *`.

### The call

```
CronCreate({
  cron:      "<M H DoM Mon *>",
  recurring: false,
  prompt:    "<the same /loop body verbatim — see kickoff template>"
})
```

Notes:
- `recurring: false` is required. The task auto-deletes after firing.
- `durable` is not needed — the next tick re-schedules itself, so a session
  death naturally pauses the loop until a human re-kicks. That is desired
  behavior.
- Jobs only fire while the REPL is idle. Make `CronCreate` your **last
  action**, then exit immediately.
- Max **50** scheduled tasks per session. Since you only ever have one
  pending tick at a time, this is irrelevant in normal operation.

### Why ~1 minute?

- Long enough that the previous tick has fully exited and the REPL is idle
  before the next one fires.
- Short enough to feel like an active loop.
- Off the prompt-cache 5-minute TTL by a comfortable margin in either
  direction; not in cache hot-path territory anyway since each tick is a
  fresh session.

If a stage is genuinely heavyweight and you want a longer pause, bump to
2–3 minutes. Don't go below 1 — you risk the next tick firing before this
one's exit completes.

## If you cannot schedule

If, at the end of a tick, `CronCreate` is unavailable or fails (tool not
present, returns an error, you've hit the 50-task limit, etc.), you **MUST**:

1. **Stop. Do not silently exit.**
2. Mark the just-finished stage `[x]` in the status file if it actually
   completed (with commit + push), or `[!]` with a one-line reason if it
   didn't.
3. **Tell the user, in plain text in the chat**, exactly:
   - Which stage just completed (or failed).
   - That you could not schedule the next tick.
   - **Why** (cite the specific cause — "`CronCreate` returned: …",
     "tool not available in this session", etc.).
   - **How they should re-kick the loop** — usually:
     ```
     Continue JOB-LOOP per DOCS/JOB-LOOP.md in /home/user/code/rust/codeless-workspace;
     status file DOCS/sessions/<file> is current.
     ```
4. Exit.

A tick that finishes a stage but neither schedules nor reports DONE nor
follows this escalation is a **broken loop**.

## Memory hygiene

In-session context is per-tick now (each fresh fire is a new session), so
in-tick context only grows for the duration of one stage. The relevant
hygiene is **what carries between ticks**, and that is exclusively:

- The status file (committed and pushed).
- The git history on the remote.
- [`CLAUDE.md`](../CLAUDE.md) for cross-session preferences and the
  cross-platform rules from [SCOPE.md](./SCOPE.md).
- [`CODELESS.md`](../CODELESS.md) for project-memory entries (per
  [SCOPE.md](./SCOPE.md) — the project-memory file).

If you find yourself wanting to "remember" something for the next tick,
write it to the status file's Notes (or to `CODELESS.md` if it's a durable
project-level fact). Nothing else survives.

## Guardrails (loop halts and does NOT reschedule)

- **Dirty tree at tick start.** Means the previous tick failed mid-flight.
- **Unparseable status file.** Never guess the next stage.
- **Verification fails.** No `--no-verify`, no skipping tests.
- **Push fails.** No `--force`. Investigate auth / upstream.
- **Cross-platform rule trip.** A stage that adds a UI import of
  `@tauri-apps/api/core`, lands a process-spawn call into a non-host crate,
  or violates the crate iOS-safe / Android-safe columns halts the loop.
- **Max stages per tick.** Bounded by the batch rule in "Hard rules" #3
  (≤4 S, or 1 M + optional related S, or 1 L sub-stage). A tick that
  exceeds the budget is a bug — schedule and let the next tick continue.
- **Untagged stage.** A `[ ]` stage with no `[S|M|L]` tag halts the loop;
  sizing must be explicit before work begins.
- **Max total ticks.** Hard ceiling (e.g. 30) so a buggy loop can't run
  forever. Track ticks elapsed in the status file's Notes if you need a
  counter.
- **Human override.** Any commit on the branch by a non-loop author halts
  the loop until the status file is re-stamped.

When any guardrail trips: halt, do NOT call `CronCreate`, explain in chat.
The absence of a successor task is the kill switch.

## Failure modes & recovery

| Symptom                          | Recovery                                                  |
| -------------------------------- | --------------------------------------------------------- |
| Stage marked `[!]`               | Human reads Blockers, fixes or rewrites stage, re-kicks.  |
| Tick never woke                  | The previous tick must have exited without scheduling.    |
|                                  | Check chat output for the halt explanation, then re-kick. |
| Commit succeeded, push failed    | Halt. Resolve auth / non-fast-forward by hand, then       |
|                                  | re-kick — the loop will see clean tree + ahead-by-N.      |
| Status file and git log disagree | git log wins. Rewrite status file from commit history.    |
| Loop is "done" but goal isn't    | Add stages to the status file, flip last `[x]` back to    |
|                                  | `[ ]`, re-kick.                                           |
| Two pending tasks in `CronList`  | Means a tick scheduled twice. Delete the older one with   |
|                                  | `CronDelete`, audit the loop body for double-schedule.    |
| `cargo test` flaky on `claude`   | Tests must set `PATH` to the fake binary explicitly       |
| binary detection                 | (per SCOPE.md). Fix the test setup; do not skip the test. |

## Code best practices — non-negotiable per stage

The loop is **not** a way to bypass review-quality standards. Every stage
must read like a hand-crafted commit:

- **Read [`CLAUDE.md`](../CLAUDE.md) and the relevant section of
  [SCOPE.md](./SCOPE.md) before each stage** that touches a new area.
  Especially:
  - The crate dependency-direction rule (mobile shells depend only on
    `types + client`).
  - The `RpcClient`-only rule for UI modules.
  - The single-responsibility-per-file rule.
- **Minimum diff.** Don't refactor adjacent code "while you're there."
- **Tests live with the code.** If the stage adds logic, the same commit
  adds the test (per SCOPE.md Testing strategy).
- **No `--no-verify`, no `--force`, no skipped hooks.** If a hook fails,
  fix the cause; don't bypass it.
- **One commit per stage.** Code change + status file update folded
  together is fine; two unrelated changes in one commit is not.

### Comments are load-bearing — write them for the next agent

The next tick is a different session — the conversation is gone; the
**comments are what remains**. Treat them as the long-term spec for intent.

**Do**

- Explain **why** the code is the way it is — the constraint, the invariant,
  the surprising bit, the alternative you rejected and the reason.
- Note non-obvious assumptions a future change could violate (units, locking
  order, ownership, lifetime, ordering guarantees).
- Keep them at the **point of surprise**, not on every line.
- Match the surrounding style; one short line is usually enough, a short
  paragraph when the *why* is genuinely subtle.

**Do not**

- ❌ **No emojis.** Anywhere. Ever. Not even in TODOs.
- ❌ **No task-status / process comments.** Never reference stages, ticks,
  milestones, version tags, "added in stage 3", "TODO from M5", "for the
  parser-rewrite branch", "fixed in PR #123". The comment must still make
  sense after the loop finishes and the branch merges.
- ❌ **No restating the code.** `// increment counter` above `counter += 1`
  is noise. Delete it.
- ❌ **No decorative banners, ASCII art, dividers**, or emoji-as-icons.
- ❌ **No essays.** If you need three paragraphs, the design is probably
  wrong — fix the code or move the explanation to the doc tree.

**The test:** would a brand-new agent reading this file alone, with no chat
history, understand *why* this code is shaped this way? If yes, the comment
is doing its job. If the comment only makes sense in the context of the
current task, delete it.

## Kickoff prompt (template)

Paste into a fresh Claude Code session pointed at `/home/user/code/rust/codeless-workspace`.
**The same text becomes the `prompt` argument of every `CronCreate` call**
— each tick re-injects this verbatim into its scheduled successor. The
fully-fleshed-out version with bracketed slots lives in
[JOB-LOOP-KICKOFF.template.md](./JOB-LOOP-KICKOFF.template.md).

```
You are running JOB-LOOP per DOCS/JOB-LOOP.md.

Repo:        codeless
Branch:      <branch>
Status file: DOCS/sessions/<YYYY-MM-DD>-<slug>.md
Spec:        DOCS/SCOPE.md (the project's scope; cite it when planning)
Rules file:  CLAUDE.md (repo root; the rules an agent must follow)
Goal:        <one sentence — what "done" looks like>

Stages (ordered, each tagged [S|M|L]):
  1. [S|M|L] <…>
  2. [S|M|L] <…>
  …

Scheduler: CronCreate one-shot, ~1 min between ticks
Max ticks: 30

Batching rule (do as much as fits in ONE tick):
  - up to 4 contiguous S stages in the same area, OR
  - 1 M stage (+ optionally 1 closely-related S), OR
  - the next sub-stage of an L (split L into S/M first).
  Stop the batch on any failure or if the diff exceeds the plan.

Procedure each tick:
  - Pre-flight (clean tree, parse status file, all-done check).
  - Decide the batch from the next [ ] stages and their tags.
    Announce it in chat in one line.
  - For EACH stage in the batch, in order:
      plan -> implement -> verify (cargo test/clippy/fmt; tsc/vitest if
      UI; specta snapshot if wire types touched) -> update status file ->
      commit AND push via mani (mani run commit/push --projects codeless).
      Push is required per stage, not just at end of tick.
  - If all stages [x] -> report DONE, do NOT reschedule.
  - Else: SCHEDULE THE NEXT TICK. This is a hard requirement. Call
    CronCreate with recurring:false and a 5-field local-time cron
    expression for ~1 minute from now (pick an off-minute, not :00
    or :30). Pass this exact prompt verbatim as the `prompt` arg.
    If CronCreate is unavailable or returns an error, DO NOT exit
    silently — follow JOB-LOOP.md "If you cannot schedule": tell the
    user which stage finished, why you can't schedule, and how to
    re-kick.
  - Halt without rescheduling on any guardrail trip in JOB-LOOP.md
    (and explain why, in chat, before exiting).

If the status file does not exist yet, create it at
DOCS/sessions/<YYYY-MM-DD>-<slug>.md (use today's date and a kebab-case
slug), populate from the stages above with [S|M|L] tags, commit and push
as "stage 0: init status", then begin tick 1.
```

## Open questions

- Should the loop open a PR at the end of a phase (e.g. when all Phase 1
  stages are `[x]`), or stop at "all stages pushed on branch" and let a
  human open the PR? Suggestion: PR-on-DONE for completed phases, branch-
  only for partial / mid-phase loops.
- The mani workspace at `~/code/rust/mani.yaml` may eventually host
  multiple loops in parallel (e.g. one for codeless, one for ai-runner).
  Decide: separate status files per repo (current rule) and per-tick
  `--projects` flag, or a workspace-level status file that fans commits
  out to multiple repos. The first is what this doc assumes.
- Is there value in a `.codeless-loop.lock` file (or a row in
  `codeless.db` once Phase 2 ships) to detect concurrent loops? Probably
  yes once Codeless dogfoods itself — at that point the running Codeless
  instance can refuse to start a second loop on the same branch.
