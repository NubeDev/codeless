# Workflow — session-mutable-scope

## Sequencing

The ramp is intentionally ordered. Do not reorder:

- Stage 0a (decisions) is first, alone. A REVIEW gate follows
  immediately — wrong calls on Q4 (predicate crate) or Q7 (telemetry
  sink) cascade through stages 3-5, so catching them here is the
  cheapest possible save.
- Stage 0b (doc tightening) + H1/H3/H7 can batch — both are
  docs-and-validation work with no runtime state-machine changes,
  and both run after the decisions file is approved.
- Step 1 (REVIEW stage type) must land before Step 2 (diff-verify),
  because diff-verify is a pre-check of the REVIEW stage.
- Step 3 (predicate runner) is independent of Steps 4-6 and can land
  in parallel with the planning for Step 4 — but the REVIEW gate
  before Step 5 must confirm Step 3 is integrated before parse-time
  guards land.
- Steps 5 and 6 must land in order: parse-time guards before the
  approval CLI, so the CLI never sees malformed patches.

## Per-stage discipline

Each stage:

1. Re-reads `SCOPE.md`, this `WORKFLOW.md`, and the relevant section
   of `DOCS/SESSION-MUTABLE-SCOPE.md` before writing code.
2. Re-reads `DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md` (after stage 1
   creates it) before making any decision the doc punted on.
3. Lands code + tests in the same commit. The runtime state machine
   gets unit tests per transition; the predicate runner uses fake
   probes in-tree; integration tests use `MockRunner`.
4. Runs `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check` before the
   commit. All three must be green. `-D warnings` is non-negotiable.
5. Updates this file or `SCOPE.md` ONLY if the stage discovers a
   workflow gap. Code stages do not touch SCOPE/WORKFLOW casually.
6. Writes the handover with `done` = paths actually touched (will be
   diff-verified once Step 2 is live — practice the rule from stage
   1) and `next` = a one-sentence pointer to the next stage's first
   action.

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/session-mutable-scope`) so the
   work is recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change (e.g. an investigation stage that
only updated SCOPE.md and that doc was already current), say so
in the handover and skip the commit, but the next stage's commit
must include any side-effect files the investigation touched.

## REVIEW gate behaviour

This job has three REVIEW gates: one after stage 0a (decisions
landed), one before Step 5 (parse-time enforcement), one after Step
6 (ramp complete). Each gate:

- Commits and pushes the stage that *led* to the gate before pausing.
- Writes a handover that summarises what landed in the preceding
  stages and what the next stage will do. The user reads the handover
  to decide whether to approve.
- Does NOT advance until the user resumes. Do not preemptively start
  the next stage to "save time."

The gate after stage 0a is the cheapest gate and the highest-leverage
one — wrong Q4 (predicate crate) or Q7 (telemetry sink) cascades
silently through stages 3-5 otherwise.

The gate before Step 5 is the most important *code*-review point in
the job: it is the last moment to catch a wrong mutable-set list or
a wrong wire-format list before parse-time guards reject real patches
in production. It runs as a literal checklist (see template.yaml) —
each box is mechanical, not interpretive.

## Anti-patterns specific to this job

- **Designing a Reviewer trait, ReviewVerdict enum, or RedoBriefing
  wire type.** The scope doc collapses all of that into "REVIEW is a
  stage." If you find yourself adding one of those, you have reached
  for the prior doc's design instead of this one.
- **A new persistence store for patches.** Patches are file
  artifacts in `DOCS/SCOPE-PROPOSED.md` and approved patches are git
  commits. No new SQLite table.
- **Auto-applying patches.** Step 4 is shadow mode. Step 5 is parse
  enforcement on proposed patches. Step 6 is human approval. At no
  point does the runtime merge a patch without human action.
- **TEST stages emitting patches.** The doc rules this out with
  reasoning. Tests fail loudly for human triage; humans decide
  whether the failure surfaces a rule gap.
- **Touching wire formats (`JOB-MODEL.md`, `JOB-LOOP.md`,
  `handover.rs`) via this job's REVIEW path.** Those change via
  schema versioning. Step 0 *does* tighten `JOB-MODEL.md` prose
  directly — that is the human's edit, not a REVIEW patch.
- **Promoting prose-to-predicate aggressively.** Surface candidates
  in the patch UI; never auto-promote. Writing a predicate is itself
  a real code change.
- **Renaming or deleting predicates as a side effect of an unrelated
  stage.** Predicate deletion is its own patch type (stage 1 records
  the decision); do not improvise a deletion path.

## Run-of-show summary (for handover assembly)

| Stage | Layer | Touches |
|-------|-------|---------|
| 0a decisions | L2 | DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md (7 questions + event-naming) |
| REVIEW (post-0a) | — | confirm decisions file complete and internally consistent |
| 0b docs tightening | L2 | DOCS/JOB-MODEL.md, DOCS/JOB-LOOP.md |
| H1/H3/H7 | L1+L2 | codeless-runtime handover discovery, write-time validation |
| 1 REVIEW stage | L1 | template_runner.rs, runtime event enum (per 0a decision), rule-bearing-file list |
| 2 diff-verify | L1 | template_runner.rs pre-check phase |
| 3 predicate runner | L1 | new xtask-shaped crate (name per 0a Q4), 5 seed predicates |
| REVIEW (pre-step-5) | — | checklist gate; see template.yaml for exact boxes |
| 4 ScopePatch shadow | L2+L3 | codeless-types new wire type, ScopePatchProposed event, DOCS/SCOPE-PROPOSED.md |
| 5 parse-time guards | L1 | patch parser in codeless-runtime |
| 6 approval CLI | L2 | CLI subcommand only (UI deferred to follow-up) |
| REVIEW final | — | checklist gate; see template.yaml for exact boxes |
