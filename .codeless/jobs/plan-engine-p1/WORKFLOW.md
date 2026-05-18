# Workflow — plan-engine-p1

How to drive the stages in
[`template.yaml`](./template.yaml) against the scope in
[`SCOPE.md`](./SCOPE.md). Re-read this doc at the **top of every
stage** — it is the only thing the runtime injects that survives a
stage boundary.

## Sequencing

- Stages 1 (survey) and 2 (REVIEW) **must** complete before any
  code lands. The survey resolves SCOPE Open Questions 1–4; the
  REVIEW locks naming and the in-memory commitment.
- Stage 3 (spec data layer) and stage 4 (engine) are sequential —
  the engine imports the spec types. Do not interleave.
- Stage 5 (REVIEW) gates the tool + runtime wiring. The point of
  this REVIEW is to confirm the engine's surface area is right
  *before* it gets used from two places (`codeless.plan.*` tools
  and the schedule Action path).
- Stage 6 (tools + wiring) is the only stage that touches
  `codeless-runtime`. Keep that diff small — the engine should
  already be feature-complete after stage 4.
- Stage 7 (docs) is mandatory. P1 must update JOB-WORKFLOW.md's
  checklist so a future agent reading the doc can see what
  actually landed vs. what the plan said.

## Per-stage discipline

**Before writing any code in a stage:**

1. Re-read [`SCOPE.md`](./SCOPE.md) — confirm the stage's outcome
   sits inside scope and does not creep into P2/P3 territory.
2. Re-read the relevant section of
   [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) —
   "Integration with what already exists", "Minimal transition
   vocabulary", "(P1)", and "Naming inside the code".
3. For stages that touch a module that already exists (`schedule`,
   `email`, `outbound`), **read the existing module first**. The
   shape there is the convention; do not invent a new one.

**Before committing:**

- `cargo test -p codeless-tools` (and `-p codeless-runtime` on
  stage 6) must be green.
- `cargo clippy --workspace --all-targets -- -D warnings` must be
  green. `-D warnings` is non-negotiable.
- `cargo fmt --check` must be green.
- No new dependencies on host-only crates from inside
  `codeless-tools/src/plan/`. Re-verify with a grep for
  `tokio::process`, `std::process`, and any `codeless-runtime` /
  `codeless-adapters-host` / `codeless-server` imports in the new
  module.

## REVIEW gate behaviour

REVIEW stages still commit + push the **prior** stage's work. They
only pause the *next* stage. At each REVIEW:

- Write the REVIEW's decision into `handover.md` under a
  `## REVIEW <n> — <gate title>` heading. Include the answer to
  each SCOPE Open Question the gate resolved.
- Do not begin the next stage's work until the user has approved.

The two REVIEW gates in this template:

1. **After the survey** — locks naming (`Plan*`, never `Workflow*`),
   the linear-only transition vocabulary, and the in-memory
   commitment. Resolves Open Questions 1–4 from SCOPE.
2. **After the engine** — confirms `JobSpawner` is the only host
   coupling, the engine has no surplus tokio handles, and the
   composition `Schedule → Action → PlanEngine::start_run` will be
   one line in stage 6.

## Anti-patterns specific to this job

- **Do not invent a generic "workflow" abstraction.** The vocabulary
  is fixed: `Plan`, `PlanStep`, `PlanRun`, `PlanRunStep`,
  `Transition`. Anything named `Workflow*` is a bug.
- **Do not add `fan_out` / `fan_in` / `when:` "while you're in
  there".** Linear chain only. Drive-by DAG support is out of scope
  and out of the P1 punch list.
- **Do not reach for SQLite.** The first instinct on seeing "track
  in-flight PlanRuns across restarts" will be a table. Resist —
  that is P2. Document the limit in stage 7 instead.
- **Do not couple `PlanEngine` to a tokio runtime handle it does
  not need.** The engine reacts to events on a stream. If the
  signature ends up with `Handle::current()`, that is a smell.
- **Do not wire the engine into `codeless-runtime` before the
  REVIEW after stage 4.** The engine must be unit-testable in
  isolation first; coupling to runtime first is the easy trap
  that makes future refactors painful.
- **Do not let stage 6 grow.** If stage 6 needs to invent a new
  `JobSpawner` shape, that means stage 4 punted on the trait —
  go back to stage 4, do not paper over it in the wiring.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, and
   `cargo fmt --check`. Every command must pass. On failure: stop,
   fix, re-run; do not advance to `docs`.
2. `docs` — update `handover.md` for the next stage in the same
   worktree, so the fresh agent that opens the next stage has the
   context it needs. For REVIEW stages, include the REVIEW
   decision (see "REVIEW gate behaviour" above). Also update the
   active session doc under `DOCS/sessions/` if one is open for
   this job.
3. `git` — stage the changes (`git add -A` from the worktree root,
   or specific paths if the stage was surgical), commit with the
   message `stage N: <one-line title from template.yaml>` so the
   history mirrors the template stages one-for-one, and push to
   the job's branch (`codeless/plan-engine-p1`) so the work is
   recoverable even if the worktree is wiped. Use `mani` for
   commit + push per `CLAUDE.md`, never raw git, never `--force`,
   never `--no-verify`.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change (e.g. the
survey stage that only writes `handover.md`), mark `git` as
`skipped — no diff`, but the next stage's commit must include any
side-effect files the prior stage touched.
