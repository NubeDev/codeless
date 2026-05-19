# Workflow — assistant-fs-tools

How to drive this job. The shape is "decide the policy, then layer
the surface — schema, read tools, write tools, UI — with a parity
gate after read-only lands so the safe half is provably done
before write paths exist."

## Sequencing

- Stage 1 is **prose-only**. Pick answers to the five open
  questions in [SCOPE.md](./SCOPE.md) and record them under
  "Decisions". No code.
- Stage 3 (schema + setThreadMode RPC) is mechanical and isolated;
  it lands without any tool work.
- Stage 4 (read-only tools) is the first feature surface. Land it
  in one commit per tool so each is independently revertable.
- Stage 6 (write tools + action cards) is the risky half. Land
  `fs.write` first, exercise it through the action-card flow, then
  add `fs.edit`. The `.codeless/jobs/<name>/` route-to-`jobs.updateScope`
  special case has its own test before either is wired into the
  planner registry.
- Stage 7 (UI dropdown) is last because it cannot exist before the
  RPC; landing it earlier would mean a control that lies.

## Per-stage discipline

- Before any code change:
  - `git log -20 --oneline` for the surrounding history.
  - Read [`codeless-tools/src/tools/mod.rs`](../../../crates/codeless-tools/src/tools/mod.rs)
    and one of the existing `Tool` impls
    ([`plan_tool.rs`](../../../crates/codeless-tools/src/tools/plan_tool.rs)
    is the closest analogue) to match registration and error shape.
  - Read [`crates/codeless-runtime/src/rpc/assistant.rs`](../../../crates/codeless-runtime/src/rpc/assistant.rs)
    to confirm where tool dispatch lives and how `AssistantActionCard`
    is constructed.
- Touch only what the stage names. No drive-by refactors.
- Verify before commit:
  - **Rust**: `cargo check -p codeless-tools -p codeless-runtime -p codeless-rpc`,
    then `cargo test -p <touched crate>`, then
    `cargo clippy --workspace --all-targets -- -D warnings`.
  - **UI**: `pnpm -C codeless/ui/codeless-ui typecheck` and tests if
    they exist; otherwise visual smoke of `/assistant` showing the
    dropdown reflects server state after a refresh.
- Commit only if green. One logical batch per commit; commit
  message stage-tagged: `stage N: <one-line title>`.

## REVIEW gates

Two:

- **After stage 1** — decisions sign-off before code lands. The
  planner's tool list shape depends on decision #5 (hide vs.
  expose-and-reject); changing minds after code lands is costly.
- **After stage 4** — read-only end-to-end works in a real
  `/assistant` session against the running server. Confirm before
  write paths are added; once `fs.write` is live the blast radius
  is bigger than read-only.

Write a one-line summary into the handover at each gate. Do not
proceed.

## What "done" looks like per stage

| Stage | Done when |
|---|---|
| 1 | SCOPE.md "Decisions" section filled in; no code changed. |
| 3 | `assistant_threads.mode` migrates cleanly; `assistant.setThreadMode` round-trips via `cargo test`; existing rows default to `read-only`. |
| 4 | `/assistant` thread in `read-only` mode can list, read, and search the workspace via the planner; absolute path and `..` paths rejected by tests; six new files committed. |
| 6 | `fs.write` on an `approve-edits` thread surfaces an action card; confirm writes the file, cancel does not; `fs.write` to `.codeless/jobs/<name>/` routes through `jobs.updateScope` in every mode; bypass writes through with no card. |
| 7 | Mode dropdown in the assistant context panel persists across reloads; switching mode mid-thread takes effect on the next tool call. |

## Anti-patterns

- Spawning `rg` from `codeless-tools`. R1 — process spawn lives
  only in `codeless-adapters-host`. Use a pure-Rust walker.
- Trusting a client-reported mode at tool dispatch. The mode is
  read from the thread row server-side every call.
- A second confirmation primitive for `fs.write`. Reuse the
  `AssistantActionCard` + `confirm_assistant_action` dispatcher F3
  already built; do not invent a parallel surface.
- Caching the mode in the UI as the source of truth. R4 — the
  dropdown reflects server state.
- Letting `fs.write` write under `.codeless/jobs/<name>/`. That
  path always routes through `jobs.updateScope`; if the special
  case is missing, the paused-job rule is silently dead.
- Adding "audit log" / "undo" for bypass writes. Bypass is "I
  trust this thread"; `git` is the audit trail.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's verify list. Every step must pass.
   On failure: stop, fix, re-run; do not advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that opens
   the next stage has the context it needs.
3. `git` — stage the changes, commit with the message
   `stage N: <one-line title from template.yaml>`, and push to the
   job's branch (`codeless/assistant-fs-tools`).

A stage is not "done" until all three are green and the push
succeeds. Never `--force`, never `--no-verify`; if a hook fails,
fix the cause.
