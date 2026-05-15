# Workflow — scope-mutable-ui

## Sequencing

The five steps in `DOCS/SCOPE-MUTABLE-UI.md` are intentionally
ordered. Do not reorder:

- Stage 0 (decisions) is first, alone. A REVIEW gate follows
  immediately — wrong calls on OQ#3 (modal policy) or OQ#5 (D
  maturity vs health) cascade through stages 6, 8, 11.
- Step 1 (Dep #1 events + Surface A panel) batches: both touch
  the same `template_runner.rs` emission sites and the React
  layer. Split *inside* the step is allowed (events first,
  panel second), but the REVIEW gate runs after both.
- Step 2 (Dep #2 `raw_tail`) is its own stage. It is the
  smallest unblock with the highest downstream EV; the gate
  before Step 3 confirms the round-trip works against a real
  REVIEW stage's handover.
- Step 3 (Dep #3 RPCs + Surface B) batches similarly: RPCs and
  the React tab, in that order within the stage. The REVIEW gate
  after Step 3 is the most important review point — it confirms
  the entire per-job action loop composes end-to-end before
  Surface C ships any cross-job affordance.
- Step 4 (Dep #4 + Surface C) ships only after the post-Step-3
  REVIEW gate passes.
- Step 5 (Surface D) can run in parallel with Step 4 in
  principle, but the doc orders it last to keep the
  *action* loop (A/B/C) honest before the *reading*
  affordance (D) lands. Honour that ordering.

## Per-stage discipline

Each stage:

1. Re-reads `SCOPE.md`, this `WORKFLOW.md`, the relevant section
   of `DOCS/SCOPE-MUTABLE-UI.md`, AND the matching dependency
   block before writing code.
2. Re-reads `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md` (after Stage 0
   creates it) before making any decision the doc punted on.
3. Lands code + tests in the same commit. Rust: unit tests per
   new type and RPC; integration tests use `MockRunner` and
   `MockRpcClient` where the path crosses the trait boundary.
   React: component tests cover the three render states (pass,
   fail-with-misses, auto-fail), reducer tests for any new
   state, and at least one integration test that drives a
   surface through `MockRpcClient`.
4. Runs `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check` before
   the commit. All three must be green. `-D warnings` is
   non-negotiable.
5. For UI stages, also runs the UI test suite (`pnpm -C
   ui/codeless-ui test --run`) and the lint pass. UI stages
   that touch routes or globals manually start the dev server
   and click through the affected surface in a real browser
   before claiming done. Browser-validation notes go in
   "What you need to know."
6. Updates this file or `SCOPE.md` ONLY if the stage discovers
   a workflow gap. Code stages do not touch SCOPE/WORKFLOW
   casually.
7. Writes the handover with `done` = paths actually touched
   (will be diff-verified) and `next` = a one-sentence pointer
   to the next stage's first action. Stage 0 and later "no
   code change" stages: still write a handover, with the
   decisions file in `done`.

## Commit + push after every stage

At the end of every stage — including stages that precede a
REVIEW gate, including stages that only edit docs — the agent
MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/scope-mutable-ui`) so the
   work is recoverable even if the worktree is wiped. Push via
   the same git auth the worktree was created with; never
   `--force`, never `--no-verify`.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage
`[x]`, do not advance. If a stage genuinely produced no change
(rare; only the REVIEW gates qualify), say so in the handover
and skip the commit; the next stage's commit must include any
side-effect files the investigation touched.

## REVIEW gate behaviour

This job has four REVIEW gates: one after Stage 0 (decisions),
one after Step 1 (gate panel ships, Risk-1 stopping point), one
after Step 3 (per-job action loop), one final after Step 5
(ramp complete). Each gate:

- Commits and pushes the stage that *led* to the gate before
  pausing.
- Writes a handover that summarises what landed in the
  preceding stages and what the next stage will do. The user
  reads the handover to decide whether to approve.
- Does NOT advance until the user resumes. Do not preemptively
  start the next stage to "save time."

The gate after Stage 0 is the cheapest and highest-leverage —
wrong OQ#3 (modal policy) or OQ#5 (D health-vs-maturity split)
cascades through stages 6, 8, 11. The gate after Step 3 is the
most important code-review point: it is the last moment to
catch a wrong author-identity policy, a wrong idempotency
contract, or a wrong cross-window event shape before Surface C
ships against them.

## Anti-patterns specific to this job

- **Rendering raw transcripts in Surface A.** The existing
  handover preview already shows the full reply. The gate
  panel is a *summary* surface — pre-check outcome, verdict,
  patch counter (when Dep #2 lands), and links to the prior
  handover. Anything more turns the panel into a wall of text.
- **Auto-applying patches.** Approve is a button that emits an
  RPC that writes a human-authored commit. The runtime never
  merges a patch without a human click. Do not add an
  "auto-approve trivial patches" shortcut, do not add a
  "policy: approve all Tighten with predicates" rule, do not
  add a delay-window auto-merge.
- **A new SQLite table for patch state.** R4 is explicit. The
  patch *file* (`DOCS/SCOPE-PROPOSED.md`) plus the git history
  of approved/rejected commits are the source of truth. If you
  find yourself reaching for a `patches` table, the schema is
  wrong; reach for an event variant on the existing bus
  instead.
- **Touching `@tauri-apps/*` from any of the four new
  modules.** R2 forbids it. The shell-injection adapters
  already cover the capabilities the patch UI needs (clipboard
  for commit-sha copy, external opener for "view diff" if it
  ever escapes the app).
- **A `Patches.web.tsx` / `Patches.mobile.tsx` split.** R3
  forbids it. Responsive design covers small-screen layout
  differences.
- **Promoting CLI features to the UI without preserving the
  CLI.** The CLI is the SSH-friendly path. The UI is additive.
  Do not delete or deprecate any `codeless patches`
  subcommand; the post-Step-3 REVIEW gate verifies all four
  CLI subcommands still work after the RPCs land.
- **Letting decisions drift between stages.** All six open
  questions are answered in `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md`
  at Stage 0. Later stages cite the file; they do not re-decide.
  A stage that contradicts a recorded decision without amending
  the decisions file is a workflow failure.
- **Skipping the third (red/warning) badge state in Surface D.**
  The doc names this explicitly as the predicate-renamed
  silent-failure mitigation. Green + grey only is wrong; ship
  the third state from the first commit of Step 5.
- **Inventing a `revert_scope_patch_approval` shortcut.** The
  10s undo toast on plain-Approve uses `git revert` on the
  commit sha returned by the approve RPC. Stage 0 records the
  exact RPC surface for this; do not invent a parallel path
  that bypasses git.

## Run-of-show summary (for handover assembly)

| Stage | Layer | Touches |
|-------|-------|---------|
| 0 decisions | L2 | DOCS/SCOPE-MUTABLE-UI-DECISIONS.md (new file, six OQ resolutions) |
| REVIEW (post-0) | — | confirm decisions file matches doc's Risk + Status sections |
| 1 Dep #1 events | L1 | codeless-types/src/event.rs (two variants), template_runner.rs (emit alongside logs), specta snapshot, wire.ts regen |
| 2 Surface A panel | L2 | ui/codeless-ui/src/modules/jobs/ReviewGatePanel.tsx (new), StageDetail.tsx wiring, useEventStream consumption |
| REVIEW (post-Step-1) | — | live smscope-test run, pass + fail cases, Patches counter row omitted |
| 3 Dep #2 raw_tail | L1 | codeless-types/src/handover.rs (raw_tail field), from_markdown/to_markdown, backwards-compat serde + regression test |
| 4 Dep #3 RPCs | L1 | codeless-rpc/src/methods.rs (three args/result shapes + AlreadyResolved), codeless-runtime/src/rpc/patches.rs (new), two new event variants in codeless-types |
| 5 Surface B Patches tab | L2 | JobTabs.tsx (SystemTabId), Patches tab module, card components, undo toast, edit modal |
| REVIEW (post-Step-3) | — | end-to-end: propose patch, approve from UI, verify commit author + trailer, verify idempotency across two windows, confirm all CLI subcommands still work |
| 6 Dep #4 list_proposed_patches + Proposal timestamp | L1 | scope_patch_queue::Proposal (add timestamp), codeless-types Proposal DTO, codeless-runtime/src/rpc/patches.rs (list method) |
| 7 Surface C /patches route | L2 | app/App.tsx (route), modules/patches/ (new), filters/group-by/sort, cross-window-events wiring, global nav count badge |
| 8 Dep #5 + Surface D badge | L2 | DOCS/SCOPE.md + DOCS/CLAUDE.md (five seed annotations), Markdown viewer component (three-state render) |
| REVIEW final | — | walk every Stopping point in doc; verify R1/R2/R3/R4/R5 hold; verify deliberately-not-included list still holds; cargo + UI tests green |
