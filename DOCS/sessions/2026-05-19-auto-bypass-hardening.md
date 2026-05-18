# Session — auto-bypass-hardening

Active session doc for the `auto-bypass-hardening` job
(`.codeless/jobs/auto-bypass-hardening/`). Stage 1 lives here; later
stages append their handover paragraphs below in order.

Authoritative source-of-truth for the job is the job's own
`SCOPE.md`. This file is the per-stage running log; it must not
contradict SCOPE.md, and any decision recorded here that conflicts
with SCOPE.md is a workflow failure (per `WORKFLOW.md` §"When to
halt").

## Stage 1 — resolve open questions (DONE)

Stage 1's deliverable was the four §"Open questions" in the job
`SCOPE.md` flipped from open to resolved, each with a chosen answer
plus a *why*. No code lands in stage 1.

The resolutions are inline in `.codeless/jobs/auto-bypass-hardening/SCOPE.md`
§"Open questions (resolved in stage 1)" — that is the single source
of truth the next stages cite. Short summary so the next session
does not have to re-read the full prose:

- **Q1 — SQLite error codes -> `InfrastructureError`.** Five primary
  codes: `8 READONLY`, `10 IOERR`, `11 CORRUPT`, `13 FULL`,
  `14 CANTOPEN`. Matcher reads primary `code()` only; extended
  codes ignored. Explicit exclusion list (codes that stay
  `RunnerError`) and the deliberate carve-out for `26 NOTADB` are
  spelled out in SCOPE.md.
- **Q2 — WAL pragma timing.** `SqlitePoolOptions::after_connect`
  applies `journal_mode=WAL` + `synchronous=NORMAL` +
  `busy_timeout=5000` on every connection acquire; `:memory:`
  databases skip the `journal_mode` line only. Per-acquire is the
  contract because sqlx recycles pool connections and connection-
  scoped pragmas reset on reopen.
- **Q3 — path-shaped tokenizer rule.** A `Done`-section token is
  path-shaped iff (a) it starts with a repo-relative prefix derived
  *from the current diff's file list* (first segment + two-segment
  prefixes + literal repo-root filenames that actually appear in
  the diff), OR (b) it resolves to an existing file under the
  worktree at pre-check time. Self-updating (new top-level dirs
  work automatically); kills all four motivating-job false
  positives. Existing `looks_path_like` shape filter still runs
  first.
- **Q4 — bypass-comment thread-through wording.** Canned policy
  paragraph stays as the lead, blank line, then a triple-backtick
  fence with **no language tag** containing `Previous-stage
  failure: <wire-name>` and `Detail: <failure_detail truncated to
  400 Unicode scalars; trailing U+2026 iff truncated>`. Wire names
  are the kebab-case serde renames (`pre-check-failed`,
  `infrastructure-error`, ...). Empty/whitespace-only details omit
  the `Detail:` line; `None` failure_class omits the whole fenced
  block.

## Stage 1 — handover for stage 2

Stage 2's first concrete unit of work, per the job template and
constraint R1:

- File: `crates/codeless-types/src/stage.rs` (variant) and
  `crates/codeless-runtime/src/store/codec.rs`
  (`failure_class_label` + `parse_failure_class` round-trip).
- Add `FailureClass::InfrastructureError` between `RunnerError` and
  `ReviewPatchInvalid` (or at the end — the wire is serde
  kebab-case so position is not load-bearing). The variant's
  one-line doc comment must explain *why* the variant exists per
  CLAUDE.md R2: "the operator-visible contract that infrastructure
  failures (SQLite disk-full, IOERR, CANTOPEN, CORRUPT, READONLY)
  halt the job rather than being silently retried by the auto-
  bypass policy." No mention of storage shape.
- Regenerate the specta wire snapshot used by `wire.ts` (the
  generator command lives in the workspace Makefile — confirm in
  stage 2 before running).
- `cargo test -p codeless-types` green.

Stage 2 must **not** touch the sqlx error mapper or the
state-machine halt branch — those are stages 3 and 4 respectively.
The variant landing alone is the smallest reverting unit; keeping
the diff small makes the M-INFRA REVIEW gate at stage 5 easy to
inspect.

## Notes for later stages

- Stage 6 (precheck tokenizer): the existing tokenizer lives in
  `crates/codeless-runtime/src/diff_verify.rs::extract_paths_from_done`
  + `looks_path_like` + `path_candidates_in`. The Q3 decision
  layers a new "(a) prefix match OR (b) worktree resolves" gate
  **after** `looks_path_like` returns true, not as a replacement.
  Stage 6 needs the diff's file list at tokeniser-call time — the
  call site already has the diff, so this is a parameter widening,
  not a new fetch.
- Stage 7 (`policy_comment`): per Q4, the function widens to take
  an `Option<&PriorFailure>` second arg. The `None` case must
  return today's byte-for-byte canned paragraph so the existing
  string-pin tests stay green without edits.
- Stage 8 (bypass-comment-build path): per Q4, this is the site
  that loads `failure_class` + `failure_detail` from the prior
  stages row and threads them into `policy_comment`. Detail
  truncation (400 Unicode scalars + U+2026 marker) happens here on
  the prompt boundary; the stored value on the row is unchanged.
- Stage 10 (UI overview glyph): the `~` glyph + tooltip change is
  one branch in `StagesOverview.tsx::stageGlyph()`. The tooltip
  source is the stage row's `bypassed_reason` (existing column)
  plus the policy name from the `StageAutoBypassed` event the
  timeline already carries.
