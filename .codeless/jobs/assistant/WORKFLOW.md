# Workflow — Assistant

How to drive this job. The shape of the work is "extract a shared
component, then add a new surface on top of it, then wire features
incrementally." The risk is regressing existing chat surfaces; the
mitigation is parity-first extraction with explicit REVIEW gates.

## Sequencing

- Stage 1 (resolve open questions) is **prose-only**. Write the
  decisions into [SCOPE.md](./SCOPE.md) under "Decisions". No code.
  Stop at the REVIEW gate so the user can sign off before the
  extraction touches three surfaces at once.
- Stage 3 (extract `CommonChat`) is mechanical. Do it as a single
  branch with one commit per call-site rewire so each step is
  independently revertable.
- Stage 5 onward (RPCs, page, tools) is feature-by-feature. Land
  each behind the same `/assistant` route — partial completion is
  visible, no feature flags.

## Per-stage discipline

- Before any code change, read recent context:
  - `git log -20 --oneline` for the surrounding history.
  - `ls codeless/ui/codeless-ui/src/modules/{jobs,ai}/` to confirm
    the call sites haven't moved since SCOPE was written.
- Touch only what the stage names. No drive-by refactors. If you
  spot a real bug, leave a one-line note in the handover and keep
  going.
- Verify before commit:
  - **Rust changes**: `cargo check -p codeless-runtime -p codeless-rpc -p codeless-server`,
    then `cargo test -p <touched crate>`.
  - **UI changes**: `pnpm -C codeless/ui/codeless-ui typecheck` and
    `pnpm -C codeless/ui/codeless-ui test` if tests exist; otherwise
    visual verification of the three call sites.
- Commit only if green. One logical batch per commit; commit
  message stage-tagged: `assistant: <stage N> — <summary>`.

## REVIEW stages

Two REVIEW gates: after stage 1 (decisions) and after stage 4
(extraction parity). Both pause for the user. Write a one-line
summary into the handover and stop — do not proceed to the next
stage until a human approves.

## What "done" looks like per stage

| Stage | Done when |
|---|---|
| 1 | SCOPE.md "Decisions" section filled in; no code changed. |
| 3 | `CommonChat` exists; `RunPane`, `JobChatPage`, AI panel all render through it; existing tests pass; manual smoke of all three surfaces matches old behaviour. |
| 5 | New tables migrate cleanly; `assistant.*` RPCs round-trip via `cargo test`; no UI yet. |
| 6 | `/assistant` loads, can create + delete a thread, no-op responder echoes a reply. |
| 7 | Each manage tool surfaces as an action card; confirm/cancel both work; cancel mutates nothing server-side. |
| 8 | Draft-from-conversation produces a `DraftJob`; confirm creates the job; cancel discards the draft. |
| 9 | Inline diff edits apply via `jobs.updateScope`; running-job edit forces a pause prompt; open-in-editor opens the file. |

## Anti-patterns

- Adding a parallel `Assistant.web.tsx` / `Assistant.mobile.tsx`.
  R3 — one responsive component.
- Reaching past `RpcClient` for "just this one fetch". R2.
- Caching authoritative chat state on the client. R4.
- Trusting the `kind` prop on `CommonChat` for capability checks.
  Capabilities live on the server.
