# JOB-LOOP kickoff prompt — reusable template (Codeless)

Fill in the bracketed bits and paste the fenced block below into a fresh
Claude Code session pointed at `/home/user/code/rust/codeless-workspace`. Spec:
[JOB-LOOP.md](./JOB-LOOP.md). Project scope: [SCOPE.md](./SCOPE.md).

Quick checklist before pasting:

- Pick a date + kebab-case slug for the status file (e.g.
  `2026-05-12-phase-1-crate-skeleton`). Files live at
  `DOCS/sessions/<YYYY-MM-DD>-<slug>.md`.
- Pick a branch name. For SCOPE-driven work this is usually
  `feat/phase-<N>-<slug>` or `feat/<slug>`.
- Tag every stage `[S]`, `[M]`, or `[L]`. `L` stages must be split into
  S/M sub-stages before the loop touches them.
- Keep the goal to one sentence. Detail belongs in the linked SCOPE.md
  section.
- Confirm `CLAUDE.md` exists at the repo root (Phase 1 deliverable per
  SCOPE.md). If not, the first stage of the kickoff should create it.

## The prompt

```
You are running JOB-LOOP per DOCS/JOB-LOOP.md.

Repo:        codeless
Branch:      <branch name, e.g. feat/phase-1-skeleton>
Status file: DOCS/sessions/<YYYY-MM-DD>-<slug>.md
Spec:        DOCS/SCOPE.md   # the project's scope; cite it when planning
Rules file:  CLAUDE.md       # repo root; rules an agent must follow
Phase:       <Phase 1|2|3|4|5|6|7 from SCOPE.md, optional but useful>
Goal:        <one sentence: what "done" looks like for this loop run>

Stages (ordered, each tagged [S|M|L]):
  1. [S|M|L] <…>
  2. [S|M|L] <…>
  3. [S|M|L] <…>
  …

Sizing reminder:
  S = mechanical, ≤ ~15 min, low risk.
  M = real thinking, one focused area.
  L = MUST be split into S/M sub-stages before being worked.

Scheduler: CronCreate one-shot, ~1 min between ticks
Max ticks: 30

Batching rule (do as much as fits in ONE tick):
  - up to 4 contiguous S stages in the same area, OR
  - 1 M stage (+ optionally 1 closely-related S), OR
  - the next sub-stage of an L.
  Stop the batch on any failure or if the diff exceeds the plan.

Procedure each tick:
  - Pre-flight: clean tree, parse the status file, all-done check.
  - Decide the batch from the next [ ] stages and their tags.
    Announce it in chat in one line ("batch: stages X, Y").
  - For EACH stage in the batch, in order:
      plan -> implement (minimum diff, follow CLAUDE.md and the
      relevant SCOPE.md section) ->
      verify:
        Rust:  cargo test --workspace, cargo clippy -D warnings,
               cargo fmt --check
        TS UI: tsc --noEmit, vitest (if logic added)
        Wire types: specta snapshot test (git diff --exit-code on
                    generated TS)
        Runner/PTY tests: explicit PATH to the fake binary, never
                          trust host claude install
      ->
      update the status file (check box, move ← next, bump
      Last tick + Current stage, Notes line if non-obvious) ->
      commit AND push via mani:
        ./bin/mani --config mani.yaml run commit --projects codeless MSG='stage N: <title>'
        ./bin/mani --config mani.yaml run push --projects codeless
      Push is required per stage, not just at end of tick.
  - If all stages [x] -> report DONE, do NOT reschedule.
  - Else: SCHEDULE THE NEXT TICK. Call CronCreate with
    recurring:false and a 5-field local-time cron expression for
    ~1 minute from now (pick an off-minute, not :00 or :30). Pass
    this exact prompt verbatim as the `prompt` arg.
    If CronCreate is unavailable or returns an error, DO NOT exit
    silently — follow JOB-LOOP.md "If you cannot schedule": say
    which stage finished, why scheduling failed, and how to re-kick.
  - Halt without rescheduling on any guardrail trip in JOB-LOOP.md
    (untagged stage, dirty tree, verify fail, push fail, cross-platform
    rule violation — UI imports of @tauri-apps/api/core, process-spawn
    in a non-host crate, mobile-unsafe code in a mobile-reach crate)
    and explain why in chat before exiting.

If the status file does not exist yet, create it at
DOCS/sessions/<YYYY-MM-DD>-<slug>.md, populate from the stages above
with [S|M|L] tags, include the AGENT REMINDER block from
JOB-LOOP.md "The status doc" section verbatim, commit and push as
"stage 0: init status", then begin tick 1.
```

## Worked example — Phase 1 crate skeleton kickoff

A real example you can adapt. Paste, edit slugs/dates as needed.

```
You are running JOB-LOOP per DOCS/JOB-LOOP.md.

Repo:        codeless
Branch:      feat/phase-1-skeleton
Status file: DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md
Spec:        DOCS/SCOPE.md
Rules file:  CLAUDE.md
Phase:       Phase 1 — Core skeleton + transport rule + thinnest possible run
Goal:        Land the full crate split, in-process RPC, specta codegen,
             initial sqlx migration, MockRunner-driven runtime skeleton,
             tracing baseline, and CLAUDE.md — all green and pushed.

Stages (ordered, each tagged [S|M|L]):
  1. [S] Cargo workspace + crate stubs (codeless-types, -rpc, -runtime,
         -adapters-host, -server stub, -client, -cli, -tauri-desktop stub)
  2. [S] codeless-types: Repo/Job/Stage/Task/Event/Review structs (serde)
  3. [M] codeless-rpc trait + in-process implementation
  4. [S] specta wire-type generation + snapshot test
  5. [S] sqlx initial migration matching SCOPE.md Appendix A
  6. [M] codeless-runtime state-machine skeleton + MockRunner test harness
  7. [S] tracing-subscriber JSON-to-stdout baseline
  8. [S] CLAUDE.md at repo root capturing the rules from SCOPE.md
  9. [S] codeless secrets set/get/rm/list against chmod 600 secrets.toml
 10. [S] Worktree manager: git worktree add/remove + reaper-on-startup
 11. [M] codeless run --once --repo <r> "<prompt>" end-to-end against
         a chosen runner, streaming events to stdout

Sizing reminder:
  S = mechanical, ≤ ~15 min, low risk.
  M = real thinking, one focused area.
  L = MUST be split into S/M sub-stages before being worked.

Scheduler: CronCreate one-shot, ~1 min between ticks
Max ticks: 30

Batching rule (do as much as fits in ONE tick):
  - up to 4 contiguous S stages in the same area, OR
  - 1 M stage (+ optionally 1 closely-related S), OR
  - the next sub-stage of an L.
  Stop the batch on any failure or if the diff exceeds the plan.

Procedure each tick:
  - Pre-flight: clean tree, parse the status file, all-done check.
  - Decide the batch from the next [ ] stages and their tags.
    Announce it in chat in one line ("batch: stages X, Y").
  - For EACH stage in the batch, in order:
      plan -> implement (minimum diff, follow CLAUDE.md and SCOPE.md) ->
      verify (cargo test/clippy/fmt; tsc/vitest if UI; specta snapshot
      if wire types touched) ->
      update the status file (check box, move ← next, bump
      Last tick + Current stage, Notes line if non-obvious) ->
      commit AND push via mani:
        ./bin/mani --config mani.yaml run commit --projects codeless MSG='stage N: <title>'
        ./bin/mani --config mani.yaml run push --projects codeless
      Push is required per stage, not just at end of tick.
  - If all stages [x] -> report DONE, do NOT reschedule.
  - Else: SCHEDULE THE NEXT TICK. Call CronCreate with
    recurring:false and a 5-field local-time cron expression for
    ~1 minute from now (pick an off-minute, not :00 or :30). Pass
    this exact prompt verbatim as the `prompt` arg.
    If CronCreate is unavailable or returns an error, DO NOT exit
    silently — follow JOB-LOOP.md "If you cannot schedule".
  - Halt without rescheduling on any guardrail trip in JOB-LOOP.md
    and explain why in chat before exiting.

If the status file does not exist yet, create it at
DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md, populate from the
stages above with [S|M|L] tags, include the AGENT REMINDER block from
JOB-LOOP.md "The status doc" section verbatim, commit and push as
"stage 0: init status", then begin tick 1.
```

## Tips for picking stages from SCOPE.md

- **Phase 1 maps cleanly to ~10–12 stages.** See worked example above.
- **Phase 2 is bigger.** Split the scheduler/queue/caps work into 3–4
  M stages and the cost+wall-clock cap work into 2 S stages. Don't bundle.
- **UI work has its own kickoff template.** Phase 3 (browser MVP) wants
  a separate loop run with a separate status file — UI verify steps
  (`tsc --noEmit`, `vitest`) differ enough from Rust verify that mixing
  them in one batch tends to thrash the tick budget.
- **Always include CLAUDE.md as an early stage** if it doesn't exist in
  the repo yet. Every later stage benefits from agents being able to
  reference it.
- **`L` stages signal "you didn't plan enough."** If a stage *should* be
  `L`, your first tick is an `S` planning stage that splits it into S/M
  sub-stages in the status file and exits.
