# Scope — auto-bypass-hardening

This job tightens the Surface F (auto-bypass) loop on the back of two
real jobs (`01KRX4ZPF10J3QZ35R5GK8336X` plugin-substrate,
`01KRXH0RYTT6EYGF435WPQS70Q` job-chat) that both completed only
because the policy auto-bypassed a chain of failures the operator
could not see in the overview UI. The recovery worked; the **feedback
loop back to the operator** did not, and one of the recovered
failures was a SQLite infrastructure error (`code 13` = `SQLITE_FULL`)
that should never have been retried in the first place.

Authoritative references:

- [`DOCS/AUTO-BYPASS-DECISIONS.md`](../../../DOCS/AUTO-BYPASS-DECISIONS.md) — current policy + thrashing-guard contract.
- [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) §"TODO — precheck rules reference" — the diff-verify rule that fires the false positives.
- [`DOCS/JOB-UI.md`](../../../DOCS/JOB-UI.md) — the stages-overview surface the new glyph lives on.
- [`crates/codeless-runtime/src/template_runner.rs`](../../../crates/codeless-runtime/src/template_runner.rs) — `classify_stage_failure` (line ~1956) and the three `RunnerError` emit sites.
- [`crates/codeless-runtime/src/auto_bypass_guard.rs`](../../../crates/codeless-runtime/src/auto_bypass_guard.rs) — Q1 thrashing-guard; not changed by this job, only referenced.
- [`crates/codeless-runtime/src/auto_bypass_policy.rs`](../../../crates/codeless-runtime/src/auto_bypass_policy.rs) — `policy_comment` is the prompt-thread-through site.

## Goal

After this job:

1. A SQLite infrastructure failure (`SQLITE_FULL` / `CANTOPEN` / `IOERR` /
   `CORRUPT`) halts the job with a structured `stop_reason` instead of
   being auto-bypassed. The policy never silently retries an
   infrastructure error.
2. The runtime SQLite pool runs in WAL mode with a 5s busy timeout,
   shrinking the failure surface that produced the original
   `SQLITE_FULL` (smaller write fanout, no journal rewrite per txn).
3. The pre-check diff-verify tokenizer no longer flags non-path
   tokens. The four real-world false positives (`tool.call`,
   `rest_proxy.path`, `metadata_json.delivery.slack`,
   `/home/user/.codeless/worktrees/ai-runner/Cargo.toml`) stop firing;
   genuine missing paths still do.
4. When auto-bypass advances a job, the next stage's prompt prefix
   carries the **prior stage's `failure_class` + `failure_detail`** so
   the model knows what tripped before and can avoid the same trap.
5. `StagesOverview` distinguishes "failed-and-bypassed" from
   "hard-failed" with a `~` glyph + tooltip carrying the policy and
   the failure detail; the operator sees recovery at a glance instead
   of having to drill into `StageDetail` to find out the job is fine.

## In scope

- New `FailureClass::InfrastructureError` variant in `codeless-types`,
  with the wire name `infrastructure-error`, plus codec round-trip in
  `store/codec.rs`.
- sqlx error -> `FailureClass` mapper at every existing `RunnerError`
  emit site in `template_runner.rs`. The set known today is lines
  `1051`, `1494`, `1579`; a grep at stage 1 confirms whether there are
  others.
- `classify_stage_failure` short-circuit to `FailureAction::Halt` for
  `InfrastructureError`, mirroring the existing `stop_reason.is_some()`
  short-circuit. Halt writes a structured `stop_reason` so the UI can
  label it (rather than the current generic "crash" wording).
- `SqliteStore` pool `after_connect`: `PRAGMA journal_mode=WAL`,
  `PRAGMA synchronous=NORMAL`, `PRAGMA busy_timeout=5000`. In-memory
  test stores stay in default mode (WAL on `:memory:` is a no-op /
  error depending on sqlx version).
- Precheck path tokenizer in the diff-verify pre-check — stage 1
  locates it, stage 6 hardens it. New rule: a token is path-shaped
  only if it (a) starts with a repo-relative prefix derived from the
  diff's file list (e.g. `crates/`, `ui/`, `DOCS/`), OR (b) resolves
  to an existing file under the worktree at check-time.
- `auto_bypass_policy::policy_comment` — append the prior stage's
  `failure_class` + `failure_detail` (when present) to the comment
  threaded into the next stage's prompt. The existing canned guidance
  remains the lead paragraph.
- `StagesOverview.tsx` — new `~` glyph + tooltip when
  `rollup.stage.bypassed_at !== null`. The stage row type gains a
  derived `bypassed: boolean` field; no schema change.
- `JobTimeline.tsx` — confirm the existing `stage-auto-bypassed` chip
  carries `failure_class` + `failure_detail`; extend if it doesn't.
- Unit + integration tests for every change above. Specifically:
  classification round-trip for each new error code, halt-vs-bypass
  decision under each policy, the four real-world tokenizer false
  positives, the threaded comment for `pre-check-failed` and
  `review-fail`, the UI snapshot for `~`-glyph rendering.
- Doc updates: `AUTO-BYPASS-DECISIONS.md` gains a new Q for the infra
  branch; `JOB-WORKFLOW.md` precheck-rules TODO flips to a real
  documented rule; `CODELESS.md` gains the "what works" line.

## Out of scope

- Rewriting the diff-verify precheck. We are tightening the
  tokenizer, not replacing it. A fuller overhaul (e.g. parsing the
  handover's `Done` section as Markdown and only checking links /
  code spans) is a follow-up job.
- Adding new `FailureClass` variants beyond `InfrastructureError`.
  The existing set (`PreCheckFailed`, `RunnerError`, `ReviewFail`,
  `ReviewUnparseable`, `ReviewPatchInvalid`) is unchanged.
- Touching the thrashing guard (`auto_bypass_guard.rs`). Two-strikes
  rule is unchanged. `InfrastructureError` halts upstream of the
  guard, same as cap-breach.
- Surfacing infrastructure failures over Slack/Telegram differently
  from other halts. Bot adapters already render `JobFailed` with a
  reason; the new `Infrastructure` reason flows through unchanged.
- The fuller PR / commit-message-prefix work tracked in
  `JOB-WORKFLOW.md` §"TODO — commit message conventions".
- Server-side disk-pressure alerting / preemptive halt before
  `SQLITE_FULL` fires. That belongs in a monitoring job.
- Mobile-shell rendering of the new glyph (mobile shell is Phase 6).

## Constraints

- **R1** — `InfrastructureError` and the new pool pragmas live in
  host-only crates (`codeless-runtime`). `FailureClass` is in
  `codeless-types` (iOS / Android safe) — a wire variant is fine,
  process-spawning code is not.
- **R2** — UI changes are inside existing components
  (`StagesOverview.tsx`, `JobTimeline.tsx`). No new `@tauri-apps/*`
  imports. The wire type lookup goes through the existing generated
  `wire.ts`.
- **R3** — no per-shell `.tsx`. The glyph is a single component
  branch.
- **R4** — `bypassed_at` and `failure_class` already live in SQLite;
  this job adds no new columns. The UI subscribes through the
  existing event surface.
- **R5** — single trust boundary unchanged.
- Comment rule from `codeless/CLAUDE.md` R2 — every new comment
  explains *why*, never *what*. The new InfrastructureError variant
  in particular must carry a one-line doc comment explaining why it
  exists (the operator-visible fact that we never silently retry
  infra failures), not what it stores.

## Deliverables (what "done" looks like)

1. `codeless/auto-bypass-hardening` branch with one commit per stage,
   pushed via mani; commit messages follow `stage N: <title>`.
2. `cargo test --workspace` green; the new tests for classification,
   halt-vs-bypass, tokenizer, and bypass-comment thread-through are
   all in the suite.
3. `cargo clippy --workspace --all-targets -- -D warnings` green.
4. `cargo fmt --check` green.
5. `pnpm -C ui/codeless-ui lint` green; `pnpm -C ui/codeless-ui test`
   green with updated snapshots for the new glyph + tooltip.
6. Manual smoke: with the server running, force a SQLITE_FULL by
   pointing the runtime at a DB with `PRAGMA max_page_count=1`,
   submit a one-stage job, observe the job halts with
   `stop_reason=Infrastructure` and no `stage-auto-bypassed` event
   fires. (The fixture for this lives in the stage-4 unit test; the
   manual smoke is the integration confidence check.)
7. `DOCS/AUTO-BYPASS-DECISIONS.md` and `DOCS/JOB-WORKFLOW.md` updated
   per stage 12; `CODELESS.md` "What works today" gains the
   landing-line.

## Open questions (resolve in stage 1, before any code)

1. **Which SQLite error codes belong to `InfrastructureError`?**
   Bias: the four codes seen as recoverable-by-the-host —
   `13 SQLITE_FULL`, `14 SQLITE_CANTOPEN`, `10 SQLITE_IOERR`,
   `11 SQLITE_CORRUPT`. Codes that point at a runtime bug
   (`1 SQLITE_ERROR`, `21 SQLITE_MISUSE`) stay as `RunnerError`
   because they are the runner's fault, not the host's.
2. **When does the WAL pragma fire?**
   Bias: in `SqlitePoolOptions::after_connect` on every connection
   acquire, so a reconnect after pool churn re-applies the pragma.
   `:memory:` databases opt out (WAL is not meaningful there).
   Note: WAL is a per-database setting persisted in the file header;
   applying it on every connect is idempotent.
3. **What is the precise tokenizer rule for path-shaped tokens?**
   Bias: a token in `Done` is "path-shaped" only if EITHER
   (a) it starts with a repo-relative prefix derived from the diff's
   file list (the set `{crates/, ui/, DOCS/, plugins/, bin/, setup/,
   tests/, migrations/, mani.yaml, Cargo.toml, ...}` extracted from
   the diff at check-time), OR
   (b) it resolves to an existing file under the worktree.
   Anything else — dotted RPC method names, JSON key paths, config
   keys — is not path-shaped and not flagged.
4. **What exactly do we thread into the next stage's prompt on
   auto-bypass?**
   Bias: append a fenced block after the existing canned guidance:
   ```
   Previous-stage failure: <failure_class>
   Detail: <failure_detail>
   ```
   The `failure_class` is the wire name (`pre-check-failed`,
   `review-fail`, etc.). `failure_detail` is the raw string already
   stored on the row, truncated to 400 chars at the prompt boundary
   to keep the per-stage prefix short. The threading uses the same
   prompt-prefix assembler the existing operator comment uses; we
   are extending the comment, not adding a new section.

Record the chosen answer + one-line *why* under each in this file
during stage 1; no implementation code in stage 1.

## References

- Two failed-and-recovered jobs that motivated this:
  `01KRX4ZPF10J3QZ35R5GK8336X` and `01KRXH0RYTT6EYGF435WPQS70Q` in
  the events table — read the `stage-completed{failed}` +
  `stage-auto-bypassed` pairs to see what the policy did and what
  the operator could not see.
- Wire types: `codeless-types::FailureClass`,
  `codeless-types::Event::StageAutoBypassed`,
  `codeless-types::StageRow`.
- The thrashing guard contract (unchanged):
  [`auto_bypass_guard.rs`](../../../crates/codeless-runtime/src/auto_bypass_guard.rs).
- The Surface F decisions doc (this job adds a Q to it):
  [`DOCS/AUTO-BYPASS-DECISIONS.md`](../../../DOCS/AUTO-BYPASS-DECISIONS.md).
