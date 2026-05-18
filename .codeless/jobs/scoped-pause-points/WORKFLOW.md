# Workflow — scoped-pause-points

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md`.

## Sequencing

- Stage 1 is design-only — **no code commits**. It edits `SCOPE.md`
  (resolving the four open questions) and creates
  `DOCS/SCOPED-PAUSE-POINTS.md` with the grammar + rejection rules.
- Stage 2 is a REVIEW gate. Do not start stage 3 until approved.
- Stages 3-6 may not be batched. Each ships its own commit so the diff
  is a coherent unit and a revert is one commit.
- Stage 7 is the mid-job REVIEW gate (server-side complete). Do not
  start stage 8 until approved.
- Stage 8 is the UI work. One commit.
- Stage 9 is the final REVIEW gate. Do not auto-merge.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something outside §"In scope", stop and surface it — don't
   silently expand scope.
2. Re-read the relevant section of `DOCS/SCOPED-PAUSE-POINTS.md` (after
   it lands in stage 1). The doc is the grammar reference for the
   parser; the SCOPE is the brief.
3. Check the R1 boundary:
   `rg 'tokio::process|std::process' crates/ --type rust`
   The match set outside `codeless-adapters-host` must not grow.
4. Check the R2 boundary in the UI stage:
   `rg '@tauri-apps/api' ui/codeless-ui/src/ -g '!src/shells/**'`
   Must not grow. The divider chip and chat marker read through
   `RpcClient` only.

Before committing a stage:

1. `cargo test --workspace` green.
2. `cargo clippy --workspace --all-targets -- -D warnings` green.
3. `cargo fmt --check` green.
4. The stage's test(s) actually exercise the new behaviour. Parser
   tests must assert *which* `ScopeError` variant fires, not just that
   some error fired. Runtime test must walk the mock runner through at
   least one full pause-resume cycle, not just assert state.
5. Update `SCOPE.md` §"Deliverables" with `[x]` against anything
   completed in the stage.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## REVIEW gates

Three gates: stage 2 (design), stage 7 (server-side complete),
stage 9 (UI complete).

At each REVIEW gate, write a handover comment in the job chat with:

- **Stage 2:** the chosen answer for each of the four open questions
  with one-line *why*; a diff-link line for `DOCS/SCOPED-PAUSE-POINTS.md`
  showing the grammar + rejection rules; a worked-example block
  rendering a `pause_points:` list against a sample stages list and
  showing the resolved ordinals.
- **Stage 7:** `cargo test --workspace` tail; the list of new wire
  types and the new `StopReason` variant; a one-paragraph note on the
  runtime hook placement (where in the state-machine transitions the
  check fires) and the resync rebuild semantics.
- **Stage 9:** a screenshot or terminal capture of the Playwright run;
  a note on any open follow-ups (edit-from-UI, recurring points, etc.)
  to file as separate jobs.

Do not proceed past a REVIEW gate without explicit approval in chat.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's `verify:` list (or `verify_cmd`).
   Every step must pass. On failure: stop, fix, re-run; do not
   advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that opens
   the next stage has the context it needs. For stages that touch
   the grammar (1, 3, 4), update `DOCS/SCOPED-PAUSE-POINTS.md` in
   the same commit.
3. `git` — stage the changes (`git add -A` from the worktree root, or
   specific paths if the stage was surgical), commit with the message
   `stage N: <one-line title from template.yaml>`, and push to
   `codeless/scoped-pause-points`.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change, say so in the
handover and mark `git` as `skipped — no diff`.

## Anti-patterns specific to this job

- **Do not** introduce a new pause primitive. Stage 6's runtime hook
  must call the existing `pause_job` entry point, not duplicate its
  logic. The whole point of this job is that the schedule sits *on
  top of* the existing state machine.
- **Do not** add `position: String` or `target: String` on the wire.
  Use typed enums (`PausePointPosition`, `PausePointTarget`,
  `TodoSelector`). Stringly-typed wire is forbidden — the parser must
  fail at submit, not the runner at transition.
- **Do not** silently no-op an unresolvable point. Submit-time
  resolution failures are typed `ScopeError`; the job stays in
  `draft` with the error visible. The only valid silent no-op is a
  point whose target stage already completed before a `resync` added
  it (open question 3) — and even that emits a note in the resync
  event payload.
- **Do not** wire UI editing of points in this job. Render-only. The
  Playwright test should not click any edit affordance; if one exists,
  the SCOPE was misread — surface it instead of building it.
- **Do not** key `scheduled_pause_points` on `(stage_ordinal,
  todo_ordinal)`. Stage and todo ordinals are mutable across
  `resync`; the row's primary key is `(job_id, ordinal)` where
  `ordinal` is the point's declared order in `pause_points:`.
  Resolution happens at trigger time, not at row-insert time, so a
  resync that renumbers stages does not orphan rows.
- **Do not** treat `StopReason::ScopedPausePoint` as a terminal stop.
  It is a resumable pause — `resume_job` must advance past it the
  same way it advances past a manual `pause_job`.
- **Do not** rename the closing trio's `TodoKind` variants to make
  them "easier targets". The selector must accept the existing names
  (`checks`, `docs`, `git`); the trio is load-bearing per `JOB-UI.md`.

## When to halt

- A stage's work turns out to require a decision that wasn't in stage
  1's resolved list: stop, surface the decision in chat, do not
  silently choose.
- `cargo test --workspace` fails after a real fix attempt and you
  can't see the next move: mark the stage `[!]` in `SCOPE.md` and
  stop. Do not commit a partial implementation with a TODO.
- Any R1 grep regression (new `tokio::process` outside
  `codeless-adapters-host`) or R2 grep regression (new `@tauri-apps/*`
  outside `src/shells/**`): halt and rework the layering. R1 and R2
  are not negotiable.
