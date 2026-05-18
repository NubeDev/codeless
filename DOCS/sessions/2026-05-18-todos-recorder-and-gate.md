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
   TodoUpdated events — this commit.
6. [ ] UI rendering — Stages tab shows todo rows per JOB-UI.md,
   glyphs driven by todo events.
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

## Open questions carried forward

(unchanged from SCOPE.md §Open questions; stage 6 will need to
decide whether the UI shows `runner` items below or interleaved with
trio rows — current bridge sets ordinal = array index from 0, so
runner rows sort naturally above the trio's `u32::MAX - 2` base.)
