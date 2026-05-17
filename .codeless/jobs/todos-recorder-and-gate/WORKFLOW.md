# Workflow — todos-recorder-and-gate

## Sequencing

Stages 1 → 2 → 3 can batch into one tick each but never across
stages. Stage 4 is a REVIEW gate. Stage 5 (runner integration) can
land in parallel with stage 6 (UI) only if the agent splits them
into two distinct ticks; otherwise serial. Stage 7 is the final
REVIEW + smoke.

## Per-stage discipline

At the top of every stage, re-read:

- The stage's entry in `template.yaml` (this directory)
- The relevant section of [`DOCS/SCOPE.md`](../../DOCS/SCOPE.md)
  — the `Todo` row at L48 and the SSE event list at L818
- The matching section of [`DOCS/JOB-UI.md`](../../DOCS/JOB-UI.md)
- The existing tests in
  [`crates/codeless-runtime/src/store/todos.rs`](../../crates/codeless-runtime/src/store/todos.rs)
  — they encode the contract for `update_todo_status` timestamp
  behaviour and `trio_resolved` resolution rules. Do not change
  those tests; if a new behaviour needs different semantics, write
  new tests alongside.

Write code in the smallest reasonable chunk per tick. R3 (one
concept per file): if a stage's work would naturally live in two
files, split.

## Closing trio — the last three todos of every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST run
the trio in order. The codeless server is what's being modified
in this job, but the trio rules apply to the human-driven flow
identically (the runtime injection is the thing being built).

1. `checks` — `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check`. All three
   must pass. On failure: stop, fix, re-run; do not advance.
2. `docs` — update `handover.md` for the next stage and the active
   session doc under `DOCS/sessions/`. Anything the next stage
   needs to know goes on disk, not in chat.
3. `git` — stage the changes, commit with the message
   `stage N: <one-line title from template.yaml>` so the history
   mirrors the template, and push to `codeless/todos-recorder-and-gate`.

Never `--force`, never `--no-verify`. If a hook fails, fix the
cause.

## REVIEW gate behaviour

Stage 4 (between gate plumbing and runner integration) and stage 7
(final smoke) are REVIEW gates. Each one commits and pushes the
stage that led to the gate before pausing. In the handover, write:

- What the gate is asking the reviewer to confirm — for stage 4,
  the gate's behaviour under three failure cases (trio incomplete,
  trio failed, trio resolved); for stage 7, the end-to-end smoke
  result.
- The exact command the reviewer should run to reproduce.

## Anti-patterns specific to this job

- **Do not emit trio events from the runner.** The runtime is the
  injector. If the runner happens to also emit a `git` todo, the
  recorder should ignore it as a duplicate via the
  `(task_id, ordinal)` UNIQUE constraint — but the design intent
  is that the runner never tries.
- **Do not bypass `trio_resolved` when the gate fails.** If the
  gate refuses `StageCompleted`, the right fix is to investigate
  which trio row is unresolved, not to weaken the gate.
- **Do not re-derive the trio kinds list.** Use
  `TodoKind::TRIO` so reordering or renaming the closing items
  happens in one place.
- **Do not store todo titles in the event payload longer than
  ~200 chars.** Truncate at the emit site. The UI's row is one
  line.
