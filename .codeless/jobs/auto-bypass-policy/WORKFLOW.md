# Workflow — auto-bypass-policy

## Sequencing

The stages are intentionally ordered:

- Stage 0 (decisions) is first, alone. A REVIEW gate follows
  immediately — Q4 (exact canned comment wordings) and Q6
  (cross-doc dependency on Surface E's bypass mechanism)
  cascade through later stages.
- Stages 2–4 (wire types + event variant) batch before any
  runtime-branch work; the REVIEW gate after stage 4 is the
  schema-correctness checkpoint.
- Stage 5 (the auto-bypass branch in template_runner) is the
  load-bearing implementation stage. Must be co-shipped with
  the thrashing guard (stage 6) — neither is safe without
  the other.
- Stage 7 is a REVIEW gate AND a live smoke test against the
  smscope-test repo. This is where the runtime branch earns
  its keep against real failures.
- Stages 8–10 (UI submit picker, badge, set_job_policy RPC)
  can batch in either order; the RPC unblocks the mid-job
  policy change but the badge can render against the
  submit-time field alone.
- Stage 11 (timeline render) is the smallest UI stage.
- Stage 12 is the final REVIEW gate.

## Per-stage discipline

Each stage:

1. Re-reads `SCOPE.md`, this `WORKFLOW.md`, and the relevant
   section of SCOPE-MUTABLE-UI.md before writing code.
2. Re-reads `DOCS/AUTO-BYPASS-DECISIONS.md` (after stage 0
   creates it) before making any decision the doc punted on.
3. Lands code + tests in the same commit. Unit tests per
   new type and RPC; integration tests via `MockRunner` /
   `MockRpcClient` where the path crosses the trait boundary.
4. Runs `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check` before
   the commit. All three green; `-D warnings` is
   non-negotiable.
5. UI stages also run `pnpm -C ui/codeless-ui test --run` and
   the lint pass. UI stages that change routes or globals
   start the dev server and click through the affected
   surface in a real browser before claiming done.
6. Updates this file or `SCOPE.md` ONLY if the stage discovers
   a workflow gap. Code stages do not touch SCOPE/WORKFLOW
   casually.
7. Writes the handover with `done` = paths actually touched
   (will be diff-verified) and `next` = a one-sentence pointer
   to the next stage's first action. **Be defensive about the
   diff-verify pre-check**: only list paths your COMMIT
   actually changes in `Done`. Don't list paths you only
   read (design docs, prior handovers, dependency files);
   they will trip the pre-check.

## Commit + push after every stage

Same rules as every other Codeless job:

1. `git add -A` from the worktree root (or specific paths if
   the stage was surgical).
2. Commit with `stage N: <one-line title from template.yaml>`.
3. Push to the job's branch (`codeless/auto-bypass-policy`)
   so the work is recoverable.

A stage is not done until the push succeeds. Never `--force`,
never `--no-verify`. If a hook fails, fix the cause.

## REVIEW gate behaviour

This job has four REVIEW gates: after stage 0 (decisions),
after stage 4 (wire-types schema review), after stage 7
(runtime-branch live smoke), and final after stage 13. Each:

- Commits and pushes the stage that led to the gate before
  pausing.
- Writes a handover that summarises the preceding work and
  the next stage's first action.
- Does NOT advance until the operator resumes.

The gate after stage 7 is the most important — it is the
last moment to catch a wrong auto-bypass branch (cap-breach
slipping through, thrashing guard missing, comment not
threaded) before the UI surfaces ship on top.

## Anti-patterns specific to this job

- **Per-stage auto-bypass policy.** Named as an anti-pattern
  in the design doc. The policy is one-per-job; per-stage
  policy invites micromanagement of what was supposed to be
  hands-off.
- **A "Skip everything" preset.** Tempting; wrong. The
  thrashing guard catches the bad case, but the preset list
  should not advertise the lazy default. Five presets, no
  sixth lazy one.
- **Auto-bypass on cap breach.** R5 says the operator's caps
  are sacred. The auto-bypass branch checks `stop_reason`
  BEFORE the policy. A cap breach halts regardless.
- **Storing canned comments in config.** They are `const
  &str` in source. Changing a comment is a code change with
  a PR and a code review. Config-file drift is the failure
  mode this rule prevents.
- **Letting the policy fire on stage 0.** A first-stage
  failure has no prior context; the canned comment would
  thread into stage 1 with nothing to reference. The
  runtime branch checks `stage.ordinal > 0`.
- **Auto-bypass that loses a SCOPE patch.** A REVIEW stage
  that emitted a SCOPE-PATCH block in its handover must
  surface the patch in `DOCS/SCOPE-PROPOSED.md` BEFORE the
  bypass advances. Order: `emit_from_handover` first,
  auto-bypass branch second.
- **Set-policy on a Running job.** Refuse with Conflict.
  The operator pauses, sets, resumes. Otherwise the policy
  change could race a stage transition mid-execution.
- **Persisting thrashing-guard state.** It is ephemeral; the
  driver rebuilds it from the `stages` rows on startup. A
  new SQLite column for "consecutive auto-bypass count" is
  R4 violation (file artifact vs DB state — neither, this
  is in-memory only).

## Run-of-show summary

| Stage | Layer | Touches |
|-------|-------|---------|
| 0 decisions | L2 | DOCS/AUTO-BYPASS-DECISIONS.md (new file) |
| 1 REVIEW (post-0) | — | confirm decisions file complete and internally consistent |
| 2 AutoBypassPolicy + Job column | L1 | codeless-types::policy, jobs migration, SubmitJobArgs, specta snapshot, wire.ts |
| 3 StageAutoBypassed event | L1 | codeless-types::event |
| 4 REVIEW (post-wire-types) | — | mobile-safety, schema-version, serde round-trip, migration reversibility |
| 5 auto-bypass branch | L1 | template_runner.rs stage-failed handler |
| 6 thrashing guard | L1 | template_runner.rs in-memory state + tests |
| 7 REVIEW + live smoke | — | smscope-test repo job with scripted failures |
| 8 submit-form preset picker | L2 | SubmitJobDialog.tsx + RpcClient call |
| 9 JobPage policy badge | L2 | JobPage header component |
| 10 set_job_policy RPC | L1 | codeless-runtime::rpc::jobs |
| 11 timeline auto-bypassed render | L2 | timeline component |
| 12 REVIEW final | — | walk Step 7 stopping point, R1-R5 unchanged, all green |
