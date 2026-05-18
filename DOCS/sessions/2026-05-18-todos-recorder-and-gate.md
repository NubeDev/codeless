# todos-recorder-and-gate

Branch:      codeless/todos-recorder-and-gate
Job dir:     .codeless/jobs/todos-recorder-and-gate/
Goal:        Land the runtime + UI layers for the Todo feature whose
             types, events, SQLite schema, and store methods already
             merged. Stages tab renders todo rows nested under ticks;
             the closing trio (checks/docs/git) is runtime-injected
             at stage entry; the runtime refuses to emit
             `StageCompleted` until `SqliteStore::trio_resolved`
             returns true for the stage's terminal task.

## Stages

1. [x] StageRecorder persists todo events to SQLite — landed in
   1c3a57e.
2. [x] Trio injection at stage entry plus completion gate wired
   through state_machine — landed in e394989.
3. [x] Verify, handover, and git steps emit TodoUpdated for their
   trio rows — landed in 888e16b.
4. [x] REVIEW gate — PASS recorded in the session 01KRW8D2 handover.
5. [x] Parse Claude Code TodoWrite tool calls into TodoAdded /
   TodoUpdated events — landed in the previous commit.
6. [x] UI rendering — Stages tab shows todo rows per JOB-UI.md,
   glyphs driven by todo events — this commit.
7. [ ] REVIEW before merge — end-to-end smoke with a real mock stage
   that exercises trio + UI.

## Stage 5 notes (this commit)

`crates/codeless-adapters-host/src/ai_runner_bridge.rs` gained a
`TodoWriteTracker` and two new entry points (`map_event_with_state`,
`map_todo_write`) plus the `CLAUDE_TODO_WRITE_TOOL` tool-name
constant. The `forward_events` future now owns one tracker per run;
the runtime-side adapters (`ClaudeRunnerAdapter` and the chat seam
in `codeless-adapters-host::ai_chat`) pick it up transparently
because they all call `forward_events`. No call-site changes were
needed — the public contract of `forward_events` is unchanged.

Wire model:

- A `ToolUse { name: "TodoWrite", input: { todos: [...] } }` upstream
  event is now suppressed from the generic `Event::ToolCall` path
  (so the UI doesn't see a redundant tool-call row alongside the
  structured todo events) and translated through `map_todo_write`.
- Items are keyed by position; new positions emit `TodoAdded` with
  `kind = Runner` and ordinal = array index. Status flips emit
  `TodoUpdated` (`pending → in_progress`) or `TodoCompleted`
  (`completed → Done`). The runner's `pending | in_progress |
  completed` vocabulary maps onto `TodoStatus::{Pending, InProgress,
  Done}`; `Skipped` / `Failed` are runtime-only and never appear
  here.
- Runner ordinals start at 0; the trio occupies
  `u32::MAX - 2 ..= u32::MAX` (set by `template_runner::publish_trio`)
  so the two ranges cannot collide.
- Titles are truncated at 200 chars to satisfy
  `WORKFLOW.md`'s emit-site cap.
- Out-of-scope runners (Codex, Copilot, Anthropic REST) get no
  translation: their tool calls fall through the generic `ToolCall`
  arm. The trio still fires regardless because the runtime is the
  injector.

Anti-patterns explicitly upheld:

- The runner never emits a trio row: the parser hardcodes
  `TodoKind::Runner` and there is no path to `Checks` / `Docs` /
  `Git` from the bridge.
- No new dependency on `codeless-runtime` from `adapters-host`;
  diff state lives on the forwarder's stack.

Snapshot drift: `crates/codeless-types/tests/wire.ts.snap` was
out-of-date from the foundation merge (Todo event variants were
added without regenerating the snapshot). This commit regenerates
it via `SPECTA_UPDATE=1` and removes the stale
`wire.ts.snap.actual` checked in alongside. Unrelated to the
TodoWrite parser but needed to keep the trio's `checks` step
(`cargo test --workspace`) green.

## Reproducing

```sh
cargo test -p codeless-adapters-host --lib ai_runner_bridge
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Workspace-wide `cargo test --workspace` is green when
`adapters-host`'s git-touching tests are run single-threaded;
intermittent parallel-run failures in `git_commit::tests` and
`git_changed::tests` are pre-existing (they share `$HOME`-rooted
git config) and not introduced by this stage.

## Stage 6 notes (this commit)

`ui/codeless-ui/src/modules/jobs/StagesOverview.tsx` now nests todo
rows under each tick. The reducer gained three event arms
(`todo-added`, `todo-updated`, `todo-completed`) plus a
`todoIndex: Map<TodoId, {stageId, taskId}>` so the routing on
`TodoUpdated` / `TodoCompleted` does not require the envelope to
carry `task_id` — the recorder is the only writer of the
`(todo_id → task_id)` mapping and the UI mirrors it at insert time.

Display rules per `JOB-UI.md` § "Todo rows":

- Glyphs: `○` pending, `●` in-progress, `✓` done, `!` failed,
  `~` skipped.
- Sort key is the todo's `ordinal`. The runtime emits the trio at
  `u32::MAX - 2 ..= u32::MAX` and the parser starts runner items at
  0, so the trio sorts below the runner's own list without the UI
  needing a special case.
- Trio rows get a kind-label column (`checks` / `docs` / `git`);
  runner items render their verbatim title (no codeless-side
  prettifying — `JOB-UI.md` says bad titles are a runner-prompt
  issue, not a UI issue).

Tick aggregation (the answer to the open question carried forward
from stage 5: runner items render above the trio, not interleaved):
`effectiveTaskStatus` aggregates child todos when there are any,
falling back to the event-derived status otherwise. The runtime's
terminal answer (`task-completed` → `passed` / `failed`) always
wins so recorder lag on the trailing trio row cannot demote a
finished tick back to `running`.

Wire-snapshot regen: the foundation merge added `todo-*` event
variants to `Event` but never regenerated
`ui/codeless-ui/src/lib/rpc/generated/wire.ts`, so the UI had no
typed view of the events it was supposed to render. `cargo run -p
codeless-rpc --example wire_ts` rewrites the file from the existing
specta snapshots — this stage commits the regenerated output.

Tests: `ui/codeless-ui/src/modules/jobs/StagesOverview.test.tsx`
covers the four reducer-level contracts (route todo-added under the
right task; sort the trio below runner items; route todo-updated /
todo-completed through `todoIndex` without an envelope task_id;
drop transitions for unknown todos) and the four
`effectiveTaskStatus` cases (`!` wins, `●` while in-progress, `✓`
only when all resolved, terminal task wins). One end-to-end render
test pushes events through `MockRpcClient.emit` to assert the trio
sorts last in `checks → docs → git` order and the runner row's
title renders verbatim.

## Reproducing

```sh
( cd ui/codeless-ui && pnpm install --prefer-offline )
( cd ui/codeless-ui && pnpm exec tsc --noEmit )
( cd ui/codeless-ui && pnpm test )
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Open questions carried forward

The "live chat per stage" path (`JOB-UI.md` §Stage-N detail tab)
still needs the warm-session `--continue` plumbing. Out of scope
for this stage; stage 7 is the end-to-end smoke that exercises trio
+ UI together but does not own that work.
