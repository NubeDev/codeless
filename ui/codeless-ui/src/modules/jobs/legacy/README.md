# jobs/legacy/

Files in this directory are NOT compiled (excluded in `tsconfig.json`).
They are NOT imported by any active code path. They are kept on disk
only as a reference / starting point for a future, properly-considered
redesign of the per-job UI surface.

## What's here and why

- **`ConversationPane.tsx`** — a read-only event-stream view written
  as part of the JOBS-UX Phase 2 work
  (`DOCS/JOBS-UX.md`). It renders streaming agent bubbles, collapsed
  tool-call cards, and lifecycle dividers from the event bus. It was
  briefly routed in `JobChatPage`'s Chat tab and then reverted because
  it lacks the send / attach / drag-drop / paste / `CHAT.md` persistence
  that `JobChat` (in `RunPane.tsx`) already has.

- **`Composer.tsx`** — a state-driven action bar (run / stop / resume
  with cap-bump / re-run) that was paired with `ConversationPane`.
  Wires to the `start_job` / `stop_job` / `resume_job` / `rerun_job`
  RPCs. Same revert reason.

- **`JobPage.tsx`** — an older per-job page that competed with
  `JobChatPage` for the per-job route. `JobChatPage` is the one
  actually rendered. `JobPage` was edited during Phase 2 / 3 work
  under the wrong assumption that it was the live route; its
  conversation/composer wiring is therefore dead.

## Path forward — when the new surface gets re-introduced

Do **not** add a parallel component. The right approach is to fold
the streaming / tool-call / lifecycle rendering and the state-driven
action bar **into `JobChat` in place** so there is exactly one chat
surface in the product, with no feature regressions. The files here
are a reference for the rendering shapes (message kinds, fold logic,
button state machine) but should be re-implemented inside `JobChat`,
not re-imported.

If anything in this directory is determined to be definitively
unwanted, delete the file. Do not leave commented-out code lying
around.
