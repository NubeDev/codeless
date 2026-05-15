# Workflow — fix-ai-agent

How to drive the stages defined in `template.yaml`. The deep design
is [`DOCS/ASSISTANT-SCOPE.md`](../../../../DOCS/ASSISTANT-SCOPE.md);
the brief is [`SCOPE.md`](./SCOPE.md). Read both before stage 1.

## Sequencing

The follow-ups are strictly ordered: **F2 → F3 → F1**. F3 depends on
F2 (no planner output, nothing to dispatch). F1 depends on F3 (a
footer that hands off to `/assistant` to confirm cards is only
useful when cards actually do something).

Within F3, the sub-stages are also ordered:

- **F3a** lands the two new RPCs *first*, with tests, without any
  dispatcher or planner wiring. This is the smallest reviewable
  unit and stops F3 from being one giant commit.
- **F3b** wires the dispatcher against the now-existing RPCs. Tests
  use the action-card type directly; no model in the loop yet.
- **F3c** teaches the planner to emit cards. End-to-end only after
  this lands.

Do not batch F3a+F3b+F3c into one commit. Each is one stage, one
commit, one `mani run commit`.

## Per-stage discipline

Before writing in any stage:

1. Re-read the relevant section of `DOCS/ASSISTANT-SCOPE.md`. The
   brief in `SCOPE.md` is intentionally trimmed — invariants live
   in the deep doc.
2. Grep the existing surface. F2 reuses `agent_chat`; do not
   reinvent transport. F3 reuses `confirm_assistant_action`; do not
   add a parallel confirm path.
3. Confirm R1–R5 still hold for the change you're about to make.
   In particular: any new code under `codeless-runtime` must not
   pull in process spawning; any new UI must not import
   `@tauri-apps/api`.

Before committing in any stage:

- `cargo test --workspace` green.
- `cargo clippy --workspace --all-targets -- -D warnings` green.
- `cargo fmt --check` green.
- For UI stages: the dev server starts; the affected surface
  works in the browser; no console errors.
- The session doc in `DOCS/sessions/` (open the active one) marks
  the stage `[x]` in the same commit as the code change.
- Commit and push via mani, never raw git:
  ```sh
  ./bin/mani --config mani.yaml run commit --projects codeless \
    MSG='stage N: <title>'
  ./bin/mani --config mani.yaml run push --projects codeless
  ```

## REVIEW gate behaviour

Two REVIEW gates: after F2, and after F3. At each gate write a
handover note into the session doc covering:

- What landed in the preceding stages, with code anchors.
- What the user should manually exercise to approve (the gate's
  acceptance criteria — see `template.yaml`).
- Any open question that surfaced during the work, with the
  proposed resolution and a "deferred / decided / blocked"
  marker.

Do not auto-resume after a REVIEW gate. Halt and wait.

## What "done" looks like per follow-up

- **F2 done**: sending a message at `/assistant` yields a streamed
  model reply persisted in SQLite. No action cards yet. The
  integration test for one planner turn passes without touching
  the real network (use a fake transport).
- **F3 done**: every row in `DOCS/ASSISTANT-SCOPE.md` §Capabilities
  is exercised end-to-end through an action card. The two new
  RPCs (`draftFromConversation`, `updateScope`) have tests. The
  pause-first rule for `updateScope` fires with a typed error on a
  running job.
- **F1 done**: a message sent from the footer `AiInputBar` appears
  in `/assistant`'s transcript on next render. `useChatStore`
  owns no message data (only UI presentation state) or is gone.
  A full-width action card on `/assistant` is reachable from the
  footer via the "open in /assistant to confirm" affordance.

## Anti-patterns specific to this job

- **Do not** add a third chat surface. If `CommonChat` cannot
  express something the footer needs, extend it via props/slots —
  do not fork. The footer is a `CommonChat` composer, not a
  parallel widget.
- **Do not** persist assistant message state client-side. R4
  means SQLite + `RpcClient.subscribe()` is the only path. The
  footer rendering a message before SQLite has it is a bug, not
  an optimisation.
- **Do not** wire the dispatcher to call `jobs.*` methods that
  don't exist yet. F3a lands the RPCs *before* F3b touches the
  dispatcher. If you find yourself reaching for a method that
  isn't on `RpcServer`, stop and add it in F3a first.
- **Do not** let the `kind` prop on `CommonChat` decide what tools
  the runner allows. Capabilities are derived server-side from
  the thread row (see `DOCS/ASSISTANT-SCOPE.md` §Surfaces 2). The
  `kind` prop is UI affordance only.
- **Do not** let the F2 model loop spawn processes itself. It
  calls `agent_chat`, which already lives in
  `codeless-adapters-host`'s blast radius. Process spawn outside
  that crate fails R1 — the loop halts.
- **Do not** rename or move `NOOP_ASSISTANT_REPLY` "for now" and
  forget. The F2 stage replaces it; the constant is gone by the
  end of F2.
- **Do not** add task-status comments anywhere ("// F2: new
  planner loop"). The code shape is its own evidence; commit
  messages and `DOCS/ASSISTANT-SCOPE.md` carry the history.
