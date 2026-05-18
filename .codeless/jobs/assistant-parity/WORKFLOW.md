# Workflow — assistant-parity

## Sequencing

The job runs in three contiguous blocks separated by REVIEW gates.

**Block 1 — W2 (draft_job composer wire-up).** Stages W2a and W2b
can batch into one tick each. W2a is the source edit; W2b is the
round-trip test. REVIEW gate after W2b verifies that the planner's
seeded draft → user-edits → `submit_job` path is sound before the
larger W1 refactor begins on top of it.

**Block 2 — W1 (shared CommonChat renderer).** Stages W1a–W1d are
one tick each. W1a is the load-bearing lift (streaming + Streamdown
+ tool-card chrome out of `JobChat`); W1b shrinks the wrappers; W1c
deletes `refreshTick` once the rail subscribes to the
thread-touched envelope; W1d locks parity with a render test.
REVIEW gate after W1d verifies the renderer is stable before policy
cards land in W3.

**Block 3 — W3 (auto-bypass-aware planner + cards).** Stages
W3a–W3d. W3a is the presets extraction (UI + Rust); W3b is the
planner prompt; W3c wires the `draft_job` + `update` cards; W3d
emits the failure-time `set_policy` card. Final REVIEW gate is the
end-to-end Acceptance smoke from
[`DOCS/SCOPE-ASSISTANT-PARITY.md`](../../DOCS/SCOPE-ASSISTANT-PARITY.md).

Stages do not parallelise across blocks. Inside a block, each stage
is one tick; do not batch W1a + W1b in the same commit even though
they are conceptually one refactor — the wrapper shrink is easier
to review on its own diff.

## Per-stage discipline

At the top of every stage, re-read:

- The stage's entry in `template.yaml` (this directory)
- The matching workstream in
  [`DOCS/SCOPE-ASSISTANT-PARITY.md`](../../DOCS/SCOPE-ASSISTANT-PARITY.md)
- The current state of the files that stage touches (the parity
  doc was written 2026-05-17 and the codebase may have moved)
- The existing tests in
  [`ui/codeless-ui/src/modules/jobs/composer/`](../../ui/codeless-ui/src/modules/jobs/composer/)
  and
  [`ui/codeless-ui/src/modules/chat/CommonChat.test.tsx`](../../ui/codeless-ui/src/modules/chat/CommonChat.test.tsx)
  — they encode the contract for composer field semantics and the
  PS2a `threadId` plumbing. Do not change those tests; if a new
  behaviour needs different semantics, write new tests alongside.

Write code in the smallest reasonable chunk per tick. R3 (one
concept per file): if a stage's work would naturally live in two
files, split.

## Constraint enforcement

Before every commit:

- `grep -rn "@tauri-apps" ui/codeless-ui/src/modules/{chat,assistant,jobs}` —
  any new hit fails R2.
- `grep -rn "\.web\.tsx\|\.desktop\.tsx\|\.mobile\.tsx" ui/codeless-ui/src` —
  any new hit fails R3.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo fmt --check` for the W3 stages that touch
  Rust.
- `cd ui/codeless-ui && pnpm test -- --run` for every stage that
  touches the UI.

All must be green. On failure: stop, fix, re-run; do not advance.

## Closing trio — the last three todos of every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only touch tests — the agent MUST run
the trio in order. This job is UI-heavy but the trio still applies:

1. `checks` — for UI stages: `cd ui/codeless-ui && pnpm test --
   --run`, `pnpm typecheck`, `pnpm lint`. For Rust stages (W3a,
   W3b): the workspace-wide cargo trio above. All must pass.
2. `docs` — update [`DOCS/SCOPE-ASSISTANT-PARITY.md`](../../DOCS/SCOPE-ASSISTANT-PARITY.md)
   only if a decision in this job contradicts what's recorded
   there (then update both). Update the session doc in the same
   commit as the code change. Do not edit `DOCS/SCOPE.md` from
   this job — the parity doc is the contract.
3. `git` — commit + push via mani per `DOCS/JOB-LOOP.md`. Never
   `--force`, never `--no-verify`.

Skipping the trio halts the loop.

## REVIEW gates

Three REVIEW stages:

1. **After W2b** — verify the composer round-trip is correct and
   the planner's draft_job emit shape matches what the composer
   expects. If the planner emits fields the composer doesn't know
   about (or vice versa), the gap is recorded and resolved before
   W1 starts.
2. **After W1d** — verify the shared renderer is at parity with
   the job chat in: streaming behaviour, markdown rendering,
   tool-card rendering, scroll anchoring. Run the assistant view
   and the job view side-by-side; the message-list DOM is
   identical modulo header.
3. **After W3d** — the parity doc Acceptance list runs end-to-end:
   planner can answer "what's the policy on job X?", a user can
   switch policy via a card, a stage failure under `None` policy
   surfaces a `set_policy` recommendation. Each Acceptance bullet
   is a manual smoke check before merge.

A REVIEW that finds a defect feeds back into the block — the
session doc records the defect and the stage that introduced it,
and the next tick fixes it before advancing to the next block.

## Branch + commit hygiene

- Branch: `feat/assistant-parity` (one branch for the whole job;
  the three blocks are commit-grouped, not branch-grouped).
- Commit message format follows
  [`DOCS/JOB-LOOP.md`](../../DOCS/JOB-LOOP.md): `stage <id>: <title>`
  with the trailer line.
- The session doc lives at
  `DOCS/sessions/2026-05-XX-assistant-parity.md` (created at stage
  W2a, updated every tick).

## Halt conditions

Halt the loop and mark the stage `[!]` if any of:

- `useEventStream` or `Streamdown` cannot be lifted out of
  `JobChat` without breaking the existing job-chat streaming test
  (W1a). Record what part of the chrome is wedged into `JobChat`
  beyond the doc's prediction and stop.
- The planner emits a field the composer cannot accept, *and* the
  composer cannot grow to accept it without breaking the dialog
  shell (W2a). Unlikely — the wire shape is shared by construction
  — but record if hit.
- The `AutoBypassPolicy` variant list in the runtime has changed
  since the parity doc was written and the seven-variant promise
  no longer holds (W3a). Update the parity doc + this scope doc
  before resuming.
