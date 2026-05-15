# JOB-LOOP.md — the autonomous build loop

A job is a sequence of stages. Each stage is run by exactly one
session in its own git worktree, against a snapshot of the rulebook
(`SCOPE.md`, `CLAUDE.md`, checked-in predicates). A stage either
completes — committing code, pushing the branch, and writing a
well-formed handover — or halts. There is no third state.

The wire form is `crates/codeless-runtime` (`template_runner.rs`
drives the state machine; `handover.rs` writes the handover). This
document is the prose authority for what every session must do
between "session start" and "session end", in order, with no
shortcuts.

## Session lifecycle (one stage)

1. **Read the rulebook in this order.** `CLAUDE.md` (this repo);
   `CODELESS.md` (durable memory); `SCOPE.md` for the job;
   `WORKFLOW.md` for the job; `DOCS/SESSION-MUTABLE-SCOPE-DECISIONS.md`
   if it exists; the **previous stage's handover** at
   `runs/<job_id>/handover.md`. The session that skips the prior
   handover is operating against a phantom snapshot.
2. **Acknowledge the stage.** Before writing any code, the session
   must produce a short ack: the stage title, the files it expects
   to touch, the predicates it expects to honour, and any decision
   it intends to make that the rulebook punted on. The ack lives in
   the session transcript only — it is not committed. The point is
   to force the model to plan before it edits.
3. **Implement.** Make the change. Tests live with the code (R5 of
   `CLAUDE.md`).
4. **Verify locally.** Run the three gates from `CLAUDE.md`:
   ```
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo fmt --check
   ```
   Plus any checked-in predicates relevant to the stage (once the
   predicate runner from Step 3 lands: `cargo xtask predicates`).
   **All gates must be green before handover is written.** A
   handover authored over red gates is a workflow failure.
5. **Write the handover.** Per `JOB-MODEL.md`. The diff-verify step
   (Step 2 of the ramp) will reject a handover whose `Done` paths
   do not appear in the commit's diff — so authoring the handover
   against an unflushed working tree is wasted work.
6. **Commit + push.** Per `WORKFLOW.md`: stage title in the message,
   no `--force`, no `--no-verify`. The push is what makes the work
   recoverable; an uncommitted worktree is wiped on session end.
7. **Halt.** The next stage is a fresh session. Do not preemptively
   start it.

## Stage types

Three stage types exist; their behaviour in the runner differs.

- **WORK** — the default. Edits code, tests, or non-rule docs.
  Cannot touch *rule-bearing files* (see below). Layer-1 guard
  rejects a WORK stage whose diff touches the rule set.
- **REVIEW** — a blocking gate. Emits either a `PASS` or `FAIL`
  sentinel parsed from the last assistant turn; on `FAIL` the job
  halts and the human resumes. Starting with Step 4 of the ramp,
  REVIEW stages additionally emit zero-or-one `ScopePatch` proposal
  to `DOCS/SCOPE-PROPOSED.md`. REVIEW stages may NOT directly edit
  `SCOPE.md`, `CLAUDE.md`, or any wire-format file.
- **TEST** — runs the existing test suite plus new tests for the
  feature; a failure halts the job and surfaces for human triage.
  TEST stages explicitly do **not** emit patches (out-of-scope per
  `SCOPE.md`).

## Rule-bearing files

The Layer-1 guard pins down which files only a human (or the
human-approved patch path in Step 6) may edit. WORK stages that
touch any of these auto-FAIL before the model is even invoked:

- `CLAUDE.md` (this repo + workspace).
- `SCOPE.md` for any job.
- `WORKFLOW.md` for any job.
- `DOCS/JOB-MODEL.md`, `DOCS/JOB-LOOP.md` — wire-format prose,
  changed only via `schema_version` bumps.
- `crates/codeless-types/src/handover.rs` — the wire form.
- Every file under `crates/codeless-predicates/` (once the crate
  exists per Step 3) — predicate code is human-authored.

`SCOPE-PROPOSED.md` is **not** rule-bearing; the runtime appends to
it during REVIEW stages.

## Mandatory rules

### Ack-then-code

A session must produce its acknowledgement (step 2 above) before its
first file edit. The ack is the model's plan: what it expects to
touch, which predicates it expects to honour, which decision it
intends to make. The runtime does not store the ack — its purpose is
to force the model out of "write first, think later" mode, which is
the dominant cause of drive-by refactors (CLAUDE.md R4) and
predicate violations.

Concretely: if the session's first tool call is an `Edit` or
`Write`, the session is in violation. The first non-read tool call
in a session must be either preceded by a clearly-marked plan in
the assistant transcript, or the session is operating without one.

**Why this is enforced as a written rule rather than a runtime
check.** A heuristic that scans the transcript for an "ack" string
is brittle and would itself need an evidence stage. The rule lives
here so REVIEW stages can cite it (`failed: stage edited files
before producing a plan`) and humans can spot the pattern in a
handover diff.

### Verify-before-handover

A session must run the three local gates (test / clippy / fmt) plus
relevant predicates **before** writing the handover document.
Authoring a handover whose `Done` says "tests pass" without running
the tests is a hard violation; the diff-verify step (Step 2 of the
ramp) detects only path mismatches, not unrun gates, so this rule
exists to close that hole.

The session's transcript must show the verification commands
returning success before the handover write. A REVIEW stage may
cite the absence of green-gate evidence as a FAIL reason.

**Corollary.** If a gate is red and the session cannot turn it
green in this stage, the stage is `[!]` and halts. Do not write a
handover that claims green gates on a red tree.

### One patch per REVIEW

Starting at Step 5 of the ramp, REVIEW stages emit **at most one**
`ScopePatch` proposal. Two patches in one REVIEW is a parse-time
reject; the second patch waits for its own REVIEW stage to surface
it. This keeps the patch queue's audit trail one-to-one with REVIEW
verdicts.

### No `--force`, no `--no-verify`

The session that bypasses a hook bypasses every downstream check
that depends on the hook. If a hook fails, the session fixes the
cause. A session that pushes with `--force` against a branch the
loop is using has corrupted the loop's view of history — the
runtime cannot reconcile from that state without human
intervention.

## Failure modes and recovery

- **Red gate the session cannot fix.** Stage is `[!]`. Write a
  handover whose `Done` records the abort, whose `Next` records the
  first remediation step, and whose `Open questions` records what
  decision blocked the session. Commit + push.
- **Handover validation rejects the write.** The runtime refuses to
  finalise the stage. The session repairs the handover (most
  commonly: a `Done` path the commit did not actually touch, or an
  empty `Done`/`Next` section) and retries the write.
- **REVIEW gate FAILs.** The job halts. The human reads the
  REVIEW's `ScopePatch` proposal (if any) and the prior stage's
  handover, then either approves the patch (Step 6 UX, lands as a
  human commit), reverts the offending WORK stage, or amends the
  scope and re-runs. The runtime does not auto-retry a FAILed
  REVIEW.
- **Predicate failure post-merge.** Stale predicates (per
  `DECISIONS.md` Q5) are removed only by the approving human in the
  patch-approval commit. A predicate that crashes on every diff is
  itself a bug — fix or remove in a human-authored commit, not
  through the REVIEW path.

## Out of scope for this document

- Multi-tenant or per-user permissions — single-tenant trust
  boundary per R5 of `CLAUDE.md` is unchanged.
- Different-runner reviewers (claude work / codex review). The
  REVIEW stage type is runner-agnostic; selecting a non-default
  runner is a future job.
- Auto-merge with a delay window. Explicitly out of scope per
  `SCOPE.md` for the session-mutable-scope ramp.
