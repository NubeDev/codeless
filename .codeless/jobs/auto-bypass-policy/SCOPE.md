# Scope — auto-bypass-policy

## Goal

Build Surface F from
[`DOCS/SCOPE-MUTABLE-UI.md`](../../../DOCS/SCOPE-MUTABLE-UI.md):
the per-job auto-bypass policy that lets an operator submit a job
in "I don't care, just code" mode and walk away. The runtime
auto-advances past failing gates with a canned guidance comment
threaded into the next stage's prompt; the thrashing guard halts
the job if auto-bypass cannot rescue a recurring failure class.

This job is the **hands-off complement** to the
scope-mutable-ui job's Surface E. Where E ships the manual
escape hatch (operator clicks **[Bypass and advance]**), F lets
the operator pre-decide the bypass response and not be present
when the failure happens.

The full design lives in
[`DOCS/SCOPE-MUTABLE-UI.md`](../../../DOCS/SCOPE-MUTABLE-UI.md)
under "Surface F", "Journey 5", "Dependency #7", and "Step 7".
The matching Slack command is Surface 4 in
[`DOCS/SCOPE-SLACK-INTEGRATION.md`](../../../DOCS/SCOPE-SLACK-INTEGRATION.md).
This file is the per-job brief; both docs are authoritative.

## In scope

- `AutoBypassPolicy` enum + the five canned `const &str` comments
  in `codeless-runtime::auto_bypass_policy`.
- `Job.auto_bypass_policy` column + migration.
- `SubmitJobArgs.auto_bypass_policy` field; serde, specta,
  wire.ts regeneration.
- `Event::StageAutoBypassed` variant with the doc's exact shape.
- Stage-failed branch in `template_runner` that reads the policy
  and auto-bypasses when set (and the failure is not a cap
  breach).
- Thrashing guard: two consecutive auto-bypasses without a
  `Passed` between halts the job with `stop_reason:
  AutoBypassThrashing`.
- Submit-form preset picker (5 presets + None + Custom).
- JobPage header policy badge.
- `set_job_policy` RPC for mid-job policy changes (only when not
  Running).
- `JobPolicyChanged` event for cross-window invalidation.
- Timeline render of `StageAutoBypassed` events with a distinct
  icon and the canned comment in the hover.
- Slack `@codeless policy <id> <preset|custom|none>` command
  scaffolding (the heavy lifting lives in the Slack integration
  job; this stage adds the runtime hook).
- Tests: unit tests for the policy enum, the canned comment
  helper, the auto-bypass branch (with cap-breach and
  thrashing-guard cases), the `set_job_policy` RPC's
  refuse-on-Running behaviour, and the badge's render states.
- Live smoke test against the smscope-test repo: a scripted
  failing stage under a policy, observed advancing with the
  canned comment in the next stage's prompt.

## Out of scope

- Per-stage auto-bypass policy. The doc names this as an
  anti-pattern; the policy is one-per-job. Operators who want
  per-stage control should use Surface E.
- Auto-bypass on cap breaches. Cost-cap and wall-clock-cap are
  operator-set limits the policy must respect; the runtime
  branch must refuse to auto-bypass when `Stop_reason` is
  `CostCap` or `WallClockCap`.
- A preset called "Skip everything." The thrashing guard
  catches the bad case; advertising it as a preset is wrong.
- Auto-promotion of the policy preset based on past job
  outcomes. The operator picks; the runtime does not predict.
- Slack approval flows for setting a policy. Setting the
  policy is a single-operator action under R5; no second-party
  approval.
- Re-running an auto-bypassed stage retroactively when the
  operator changes their mind. The audit trail is the audit
  trail; if the operator wants to re-do work, they submit a
  new job.
- Letting a policy fire on the FIRST stage of a job. If stage
  0 fails, there is no prior context to seed the auto-bypass
  comment against; halt as today. The runtime branch checks
  `stage.ordinal > 0` before considering the policy.
- A "policy log" page that aggregates all auto-bypasses across
  the workspace. The run log per job is enough; cross-job
  aggregation is a separate doc.
- TODO comments in committed code. Per CLAUDE.md R4, no
  half-finished implementations. Mark unfinished stages `[!]`
  and halt.

## Constraints

- **R1 (crate dependency direction).** `AutoBypassPolicy`,
  `StageAutoBypassed`, and the canned comment string getter
  (returns `&'static str`) all live in `codeless-types`
  (mobile-safe). The stage-failed-handler branch, the
  thrashing-guard state, and the `set_job_policy` RPC impl
  live in `codeless-runtime` (host-only). Grep of
  `process::Command` outside `codeless-adapters-host` must
  remain at its current count — no new spawns introduced.
- **R2 (single transport).** UI imports `RpcClient` only.
  The preset picker, the policy badge, and the timeline
  render all use the existing `useRpc` / `useEventStream`
  hooks. No `@tauri-apps/api/*` imports.
- **R3 (one UI framework).** No per-shell files. The preset
  picker is a single component, responsive.
- **R4 (SQLite is source of truth).** The policy lives on
  the `jobs` row, not a sibling table. The thrashing-guard
  state is ephemeral (rebuilt from `stages` on driver boot);
  do not persist it separately. `StageAutoBypassed` events
  flow over the existing event bus; no new persistence layer.
- **R5 (single-tenant trust).** Unchanged. The policy is set
  by the single operator; no per-user permissions, no
  second-party approval flow.
- **Wire formats sacred.** `DOCS/JOB-MODEL.md`,
  `DOCS/JOB-LOOP.md` unchanged. `codeless-types/src/handover.rs`
  unchanged (Surface F's audit data lives on `Stage`, not
  `Handover`). The `Stage` row IS being extended with
  `bypassed_at` and `bypassed_reason` — this is shared with
  Surface E (Dependency #6a in SCOPE-MUTABLE-UI.md); if those
  columns already exist when this job runs, reuse them; if
  not, add them as part of stage 2.
- **Canned comments are `const &str`.** Five strings, version-
  controlled in `codeless-runtime::auto_bypass_policy`. NOT
  config-file driven, NOT operator-editable at runtime.
  Changing a canned comment is a code change with a PR.
- **Stage 0 protects the policy from itself.** The auto-bypass
  branch checks `stage.ordinal > 0` before considering the
  policy. A job whose very first stage fails has no prior
  context to seed the canned comment against; halt as today.
- **Cap breaches always halt.** The auto-bypass branch checks
  `stop_reason` BEFORE the policy. If `stop_reason` is
  `CostCap` or `WallClockCap`, halt as today regardless of
  policy. This is the operator's safety net against a
  runaway policy.
- **Comments per CLAUDE.md R2.** No emojis, no task-status
  comments, no restatements, no decorative banners.
- **No drive-by refactors.** Stage 5 (the runtime branch) is
  the only stage that touches `template_runner`'s
  stage-failed handler. Don't fold unrelated cleanups in.

## Resolution required from "Open questions"

Stage 0 MUST resolve these into
`DOCS/AUTO-BYPASS-DECISIONS.md`. Stage 1 is a REVIEW gate that
confirms the decisions are recorded and internally consistent
before any code stage runs.

1. **Q1 — thrashing window size.** The doc says two; defend or
   contradict. Smaller (one) defeats the policy's purpose;
   larger (three+) lets buggy gates burn more cost before the
   guard fires. Pin the number.
2. **Q2 — policy-on-cap-breach.** The doc forbids it; confirm
   and pin the failure mode if any future override is
   requested.
3. **Q3 — policy on a REVIEW stage that proposed a patch.**
   The doc says save the patch via raw_tail, bypass the gate.
   Confirm the order: `emit_from_handover` runs first,
   `auto_bypass_branch` runs second.
4. **Q4 — exact canned comment strings.** The doc names five
   presets with sample wording; stage 0 finalises the exact
   strings so stage 2's `const &str` values can be locked in.
   A wrong wording here is a follow-up PR to fix.
5. **Q5 — `set_job_policy` semantics on a Running job.**
   Lean: refuse, require pause-set-resume. Confirm and document
   the error shape (Conflict variant of RpcError).
6. **Q6 — cross-doc dependency direction.** Surface F builds
   on Surface E's bypass mechanism, which is co-shipping with
   the scope-mutable-ui job. Pin which PR lands first; if the
   scope-mutable-ui Step 1 is still open at the time this job
   runs, this job's stage 5 either waits or co-opts the
   in-flight branch.

A stage that contradicts a recorded decision without amending
the decisions file is a workflow failure.

## Stop-the-job criteria

- **Stage 2 cannot reuse `bypassed_at` / `bypassed_reason`
  columns** because Surface E's bypass mechanism (which
  introduces those columns) has not yet merged. Halt and ask
  the operator whether to wait for Surface E or add the
  columns here.
- **Stage 5's auto-bypass branch breaks any existing test in
  `template_runner.rs`.** The stage-failed handler is the
  load-bearing path; a regression there means the runtime
  fails real jobs. Halt before pushing.
- **Stage 6 cannot reproduce the thrashing scenario** in a
  unit test. The thrashing guard is the safety net the
  policy depends on; without a reproduction it is unverified.
  Halt rather than ship.
- **Stage 7's live smoke test on smscope-test fails the
  thrashing scenario.** Two consecutive auto-bypasses must
  halt the job with `AutoBypassThrashing`; if it advances to
  a third, the guard is broken.

## Pointers

- Design: [`DOCS/SCOPE-MUTABLE-UI.md`](../../../DOCS/SCOPE-MUTABLE-UI.md)
  — Surface F, Journey 5, Dependency #7, Step 7.
- Slack command for this surface:
  [`DOCS/SCOPE-SLACK-INTEGRATION.md`](../../../DOCS/SCOPE-SLACK-INTEGRATION.md)
  — Surface 4.
- The runtime this sits on:
  [`DOCS/SESSION-MUTABLE-SCOPE.md`](../../../DOCS/SESSION-MUTABLE-SCOPE.md).
- The bypass mechanism F builds on:
  Surface E in SCOPE-MUTABLE-UI.md + Dependency #6a (in flight
  on the scope-mutable-ui job's Step 1).
- Stage-failed handler to extend:
  `crates/codeless-runtime/src/template_runner.rs`.
- Submit form to extend:
  `ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx`.
- Workspace rules: `../CLAUDE.md` (workspace),
  `./CLAUDE.md` (inner repo).
