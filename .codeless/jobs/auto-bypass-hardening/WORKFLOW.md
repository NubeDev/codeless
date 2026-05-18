# Workflow — auto-bypass-hardening

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the surface docs
([`AUTO-BYPASS-DECISIONS.md`](../../../DOCS/AUTO-BYPASS-DECISIONS.md),
[`JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md),
[`JOB-UI.md`](../../../DOCS/JOB-UI.md)).

## Sequencing

- Stage 1 is research-and-resolve. No code lands. The four open
  questions in SCOPE.md must end stage 1 with a written answer + a
  one-line *why*.
- Stages 2-4 are the **infra-error halt path**: types, classifier,
  state-machine integration. They land in this order because each
  builds on the previous (variant → classifier → halt branch). Stage
  4 is the load-bearing test; do not let it slip into stage 5.
- Stage 5 (REVIEW M-INFRA) is the first gate. M-INFRA must pass
  before any UI or precheck work.
- Stage 6 is the WAL pragma — small, isolated, easy to verify. Lands
  alone so the diff is one commit you can revert without touching
  classification logic.
- Stage 7 (precheck tokenizer) is independent of stages 2-6 in
  principle but lands after them so the test fixtures can reference
  the new InfrastructureError variant if needed.
- Stages 8-9 are the prompt thread-through. Stage 8 changes the
  policy text assembler; stage 9 plumbs the prior-stage row through
  so it has data to thread. They could batch, but keeping them
  separate gives a clean revert story.
- Stage 10 (REVIEW M-FLOW) gates everything below the UI line.
- Stages 11-12 are the UI: type derivation + glyph (stage 11) and
  timeline chip (stage 12).
- Stage 13 is REVIEW M-UI.
- Stage 14 is documentation + handover.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in §"In scope", **stop and surface it** in
   the job chat — do not silently expand scope.
2. Re-read the relevant authoritative doc. For Rust stages that is
   `AUTO-BYPASS-DECISIONS.md` (especially Q1 cap-breach fence — the
   new InfrastructureError sits in the same Halt branch as caps).
   For UI stages that is `JOB-UI.md`. For the precheck stage it is
   `JOB-WORKFLOW.md` §"TODO — precheck rules reference".
3. For Rust stages: re-read `codeless/CLAUDE.md` R1 (process spawn
   restriction) and R2 (comments explain *why*). The `FailureClass`
   variant in particular must carry a doc comment that explains
   *why* infrastructure errors halt — the operator-visible contract,
   not the field's storage.
4. For UI stages: re-read `codeless/CLAUDE.md` R2 / R3 and grep
   ```
   rg '@tauri-apps' ui/codeless-ui/src --glob '!src/shells/desktop/**'
   ```
   The match set must not grow.

Before committing a stage:

1. **Rust stages**: `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, and
   `cargo fmt --check` all green. Run them from the workspace root,
   not from `crates/`.
2. **UI stages**: `pnpm -C ui/codeless-ui lint` and
   `pnpm -C ui/codeless-ui test` green. Snapshot updates are
   reviewed line-by-line — never blind `-u`.
3. The stage's new tests actually exercise the new behaviour. For
   stage 3 (sqlx error mapping) every code in §"Open questions" Q1
   must have an explicit case. For stage 7 (tokenizer) the four
   real-world false positives from SCOPE.md must each have a unit
   test.
4. Update `SCOPE.md` §"Deliverables" with a `[x]` against anything
   completed in the stage.
5. The handover for the stage names the **next stage's first
   concrete unit of work** (file path + what to add), not "next
   stage should start". This is per `JOB-WORKFLOW.md` §"TODO —
   handover schema for read-only stages".

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's verify list. Rust stages = `cargo
   test --workspace` + `cargo clippy --workspace --all-targets --
   -D warnings` + `cargo fmt --check`. UI stages = `pnpm -C
   ui/codeless-ui lint` + `pnpm -C ui/codeless-ui test`. Every
   step must pass. On failure: stop, fix, re-run; do not advance
   to `docs`.
2. `docs` — update the active session doc / handover for the next
   stage, in the same worktree, so the fresh agent that opens the
   next stage has the context it needs (per SCOPE Constraint —
   anything that must survive a stage boundary is on disk, not in
   the agent's head). For this job, also update `SCOPE.md`
   §"Deliverables" with `[x]` for the completed items.
3. `git` — stage the changes (`git add -A` from the worktree root,
   or specific paths if the stage was surgical), commit with the
   message `stage N: <one-line title from template.yaml>` so the
   history mirrors the template stages one-for-one, and push to
   the job's branch (`codeless/auto-bypass-hardening`) so the work
   is recoverable even if the worktree is wiped.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change (stage 1 is
the candidate — its deliverable is the resolved §"Open questions"
in `SCOPE.md`), say so in the handover and mark `git` as
`committed handover + SCOPE.md only` with the commit short SHA.

## REVIEW gate behaviour

Three gates: stage 5 (M-INFRA), stage 10 (M-FLOW), stage 13 (M-UI).

At each gate, write a handover comment in the job chat with:

- One bullet per item the gate is checking.
- The verify-command output for the new tests landed in the
  preceding stages (paste the relevant `cargo test` lines, not the
  whole log).
- For M-INFRA: paste the manual smoke transcript from the stage-4
  unit test or its equivalent — the `stop_reason=Infrastructure`
  row must be visible, and **no** `stage-auto-bypassed` event must
  exist for the failing stage.
- For M-FLOW: paste the assembled prompt prefix for the second
  stage of the integration test (the thread-through must be
  literally visible in the prompt, not just asserted by the test).
- For M-UI: paste the vitest snapshot diff for `StagesOverview` and
  attach (in markdown render or screenshot) a job page showing both
  a bypassed stage (`~` + tooltip) and a hard-failed stage (`!`).

The REVIEW gate **commits and pushes the preceding stage's work**
before pausing — REVIEW only blocks the *next* stage from starting.

## Anti-patterns specific to this job

- **Do not** widen `FailureClass` beyond `InfrastructureError`.
  Every other failure category we have is appropriate as-is; new
  variants are a separate decision and belong in a follow-up scope.
- **Do not** change the thrashing guard. The guard's contract
  (`auto_bypass_guard.rs`) is correct; `InfrastructureError` halts
  upstream of it via `classify_stage_failure`, the same path
  cap-breach already uses. Touching the guard is out of scope.
- **Do not** apply WAL to `:memory:` databases. The pragma is a
  no-op or error there depending on sqlx version, and the test
  suite uses `:memory:` extensively. The `after_connect` hook must
  detect this and skip.
- **Do not** rewrite the precheck. Stage 7 is a tokenizer
  tightening, not a parser overhaul. If you find yourself adding a
  Markdown parser, stop and surface it — that is a separate job.
- **Do not** thread the failure detail into the **system prompt**.
  The thread-through belongs in the operator-comment block the
  existing assembler emits as part of the per-stage user message.
  Mixing it into the system prompt changes a different surface and
  is out of scope.
- **Do not** silently update tests when a test fails. If a precheck
  test fails because the new tokenizer is stricter, the *failing
  test is the bug*; if it fails because the tokenizer regressed,
  the *tokenizer is the bug*. Diagnose before editing.
- **Do not** add a UI `bypassed.tsx` component or a wrapper around
  `StagesOverview`. The glyph change is a one-branch edit inside
  the existing component. Anything more is over-engineering and
  trips R3 in spirit even if not in letter.
- **Do not** truncate `failure_detail` further than the existing
  ~200-char ceiling without checking the assembler. The
  prompt-side 400-char ceiling decided in SCOPE Q4 is a separate
  truncation applied *only* in the thread-through path; it must
  not change the stored value.

## When to halt

- Any of `cargo test` / `cargo clippy -D warnings` / `cargo fmt
  --check` fails after a real fix attempt and the next move is not
  obvious: mark the stage `[!]` in `SCOPE.md` and stop. Do not
  commit a partial implementation with a TODO.
- A grep regression on R1 (`tokio::process` or `std::process`
  outside `codeless-adapters-host`): halt and rework. R1 is
  non-negotiable.
- A grep regression on R2 (`@tauri-apps/*` outside `shells/desktop/`):
  halt and rework.
- A stage's work needs a decision not in stage 1's resolved list:
  stop, surface the decision in chat, do not silently choose. The
  whole point of stage 1 was to front-load these.
- The precheck tokenizer test in stage 7 fails on a case **not** in
  the SCOPE.md four-positive list: the new case is either a genuine
  bug-find (good — add the test, surface the new case in chat) or a
  regression in your tokenizer rule (bad — re-derive the rule, do
  not just relax it until the test passes).
