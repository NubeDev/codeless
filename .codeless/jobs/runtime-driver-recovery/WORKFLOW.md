# Workflow — runtime-driver-recovery

## Sequencing

Order matters; do not reorder:

- Stage 1 (regression test) MUST land before any fix. It is the
  proof the fix works.
- Bug 2 (worktree idempotency) before Bug 1 (driver retry). The
  retry classifier needs to know the *new* behaviour of the
  worktree manager — if create now succeeds via adoption, the
  driver only needs to retry the rarer failure cases.
- REVIEW gate before Bug 3 and state machine edges. Last chance
  to catch a wrong allowed-edge set before clients depend on
  `reset_job`.
- UI affordance can land in parallel with the liveness audit
  (stages 6 and 7 are independent).

## Per-stage discipline

Each stage:

1. Re-reads `SCOPE.md`, this `WORKFLOW.md`, and the relevant
   section of the source files cited under "Pointers."
2. Re-reads `DOCS/RUNTIME-DRIVER-RECOVERY-DECISIONS.md` (after
   stage 4 creates it) before deciding on edge cases.
3. Lands code + tests in the same commit. Per CLAUDE.md R5 in the
   inner repo: tests live with the code.
4. Runs the full check suite:
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo fmt --check`
   For UI work in stage 6 also runs `pnpm -C ui/codeless-ui
   typecheck` and `pnpm -C ui/codeless-ui lint`. All green
   before commit; `-D warnings` is non-negotiable.
5. Touches `SCOPE.md` or this file ONLY if the stage discovers a
   workflow gap that future stages must follow.

## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/runtime-driver-recovery`)
   so the work is recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change, say so in the handover and skip the
commit, but the next stage's commit must include any side-effect
files the investigation touched.

## REVIEW gate behaviour

One REVIEW gate, before the state-machine edges land (stage 4).
The gate:

- Commits and pushes the stages leading to it before pausing.
- Writes a handover summarising the regression test, the worktree
  fix, the driver retry behaviour, and any error-classifier
  surprises.
- Records the five resolution-required decisions in the decisions
  file.
- Does NOT advance until the user resumes.

## Anti-patterns specific to this job

- **Forcing worktree adoption with `--force`.** Adoption is read-
  only inspection plus an empty-no-op `git worktree add`-equivalent
  if needed. Destructive removal lives in `reset_job`, not in
  `create`.
- **Catching errors in the driver loop and pretending success.**
  Errors must be classified and either retried or transitioned to
  `Failed`. Silent swallowing is the bug that wedged us.
- **An unbounded retry loop.** Hard cap at 3 retries. After
  exhaustion, `Failed` with a recorded `stop_reason`. The user
  uses `reset_job` to try again — that is the contract.
- **A `retry_count` column on `jobs`.** Retry state is
  in-memory in the driver; SQLite reflects the *outcome*, not the
  process. If the server restarts mid-backoff, the counter resets;
  document this trade-off and move on.
- **An auto-reset daemon.** Manual reset only. An auto-reset
  daemon would hide the bug class that made `reset_job` necessary
  in the first place.
- **Process spawn migration.** Don't move `Command` calls from the
  adapters crate into the runtime to "simplify." R1 is enforced
  by grep; the violation halts the loop.
- **Drive-by refactor of `state_machine.rs`.** Add the three new
  edges and nothing else. Resist tidying the existing matches.
- **Hand-rolled timer in tests.** Use `tokio::time::pause()` and
  `advance()` for the backoff tests; never `sleep` for real.

## Run-of-show summary (for handover assembly)

| Stage | Layer | Touches |
|-------|-------|---------|
| 1 reproduce | runtime tests | `codeless-runtime` integration test under `tests/` |
| 2 worktree idempotent | adapter | `codeless-adapters-host/src/worktree.rs` + unit tests |
| 3 driver retry | runtime | `codeless-runtime/src/job_driver_loop.rs`, error classifier, backoff |
| 4 REVIEW | — | gate; decisions file lands |
| 5 reset_job RPC + edges | runtime + rpc + types | `codeless-rpc/src/methods.rs`, `codeless-runtime/src/rpc/jobs.rs`, `state_machine.rs`, `codeless-types/src/event.rs` |
| 6 UI Reset button | UI | `ui/codeless-ui/src/...job-page...` + RpcClient binding |
| 7 liveness audit | runtime | `workspace_liveness.rs` audit + no-op test |
| 8 REVIEW final | — | R1/R5 grep, regression test passes, full check suite green |
