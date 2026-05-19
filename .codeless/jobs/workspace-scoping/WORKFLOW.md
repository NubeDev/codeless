# Workflow — workspace-scoping

This is how the agent drives the stages of the `workspace-scoping`
job. Re-read this file at the top of every stage; the closing-trio
section is load-bearing.

## Sequencing

- Stages 1 and 2 (audit + REVIEW) must finish before any code lands.
  Stage 1 produces `DOCS/EVENT-PUBLISH-AUDIT.md`; the REVIEW gate uses
  that file. Do not skip ahead.
- Stages 3 and 4 (`EventFilter` + `fs.*` RPC scoping) are independent
  Rust changes. Either order works; do them in the order the
  template lists them so the diff history matches the stage history.
- Stage 5 (REVIEW server-side scoping) is the lock point for the wire
  API. After this gate, the wire shape is fixed for this job; UI
  work in stages 6 and 7 builds on it.
- Stages 6, 7, 8 (UI plumbing, deep-link router, storage audit) are
  TypeScript-only. Land them in order; stage 8 must happen before
  the final smoke (stage 9), or cross-tab leakage will dirty the
  result.
- Stage 9 is the user-facing exit criterion. If it does not pass with
  two tabs, fix the cause; do not advance to stage 10.
- Stage 10 (cleanup) only touches docs. Keep it small; it is not a
  place to sneak in refactors.

## Per-stage discipline

- **Before any code change**, re-read `SCOPE.md` Out-of-scope and
  Constraints. The audit step in stage 1 must produce a concrete
  list of publish sites, not a summary. Format:
  `crates/.../foo.rs:NN — JobStarted { repo_id ✓ } — already carries
  repo_id` or `kind=WorkspaceUnhealthy — library-scope by design`.
- **Type changes propagate.** When `EventFilter` gains variants, the
  `RpcServer` trait, the in-process transport, the HTTP transport,
  `TauriIpcClient`, the UI `RpcClient` interface, and every test
  using `subscribe` move together. A clippy-clean workspace after
  the stage is the only acceptance signal that counts.
- **Tests live with the code (R5 of inner CLAUDE.md).** Stage 3 and
  stage 4 land unit tests in the same commit. Use `MockRunner` for
  the fan-out filter test; do not stand up real adapters.
- **R1 / R2 / R3 grep on every Rust / UI stage:**
  `grep -rn 'tokio::process\|std::process::Command' crates/` returns
  zero hits outside `codeless-adapters-host`.
  `grep -rn '@tauri-apps/api/core\|@tauri-apps/api/event' ui/codeless-ui/src/`
  returns zero hits outside `src/shells/desktop/`.
  No `Foo.web.tsx` / `Foo.desktop.tsx` splits introduced.
- **UI work runs against the local server.** `pnpm -C ui/codeless-ui
  dev` is already running on `http://127.0.0.1:1420`; the codeless
  server is on `http://127.0.0.1:7777`. Both must stay up across the
  UI stages.

## REVIEW gate behaviour

This job has three REVIEW gates: stage 2, stage 5, stage 9.

- **Stage 2 (publish-site audit).** The handover lists every event
  kind and whether it carries `repo_id`. The gate asks: "is the work
  bounded by stage 3+4, or has the audit surfaced events that need
  their own job?" The user (or the assistant on their behalf) makes
  the call. Until the answer is "bounded," stage 3 does not start.
- **Stage 5 (server-side scoping locked).** The handover summarises
  the final `EventFilter` and `fs.*` API shapes. The gate asks: "is
  this the shape we want the UI to bind to?" After this gate, UI
  work can run without fear of an API rip-up mid-flight.
- **Stage 9 (end-to-end smoke).** This gate has actual user evidence
  attached: the result of two Firefox tabs, recorded in
  `DOCS/WORKSPACE-SCOPING-SMOKE.md`. If the smoke fails, the gate
  does not pass and stage 10 does not run. Failure here means a fix
  loop, not a "ship anyway" decision.

REVIEW gates still **commit + push** the stage that led to the gate.
A REVIEW gate only pauses the *next* stage.

## Anti-patterns specific to this job

- **Do not delete `EventFilter::All`.** The doc and the SCOPE.md In-
  scope section both keep it for the global log view. Deprecation is
  a separate decision.
- **Do not introduce a new "workspace_id" type.** `RepoId` is what
  `attached_workspaces` keys on; reuse it. Naming the variable
  `workspace_id` in places is fine; introducing a new wire type is
  not.
- **Do not bypass the audit.** The temptation in stage 3 is to skip
  stage 1's report and "just add `repo_id` to `EventFilter`." That
  works on the wire, but if the runtime's events lack `repo_id`, the
  filter is a lie. Stage 1 is what makes stage 3 honest.
- **Do not "fix" `fs_cwd` to return the global default if no
  `repo_id` is passed.** That preserves today's broken behaviour
  behind a backward-compat shim. New signature: `repo_id` is
  required; missing is a typed error. The UI's job is to pass it,
  not the server's job to guess.
- **Do not regress the Tauri webview path.** Stage 6 changes are in
  the shared UI tree, so `TauriIpcClient` callers see them too. Run
  the `--native-window` codepath at the end of stage 7 to confirm it
  still loads, even though browser tabs are the focus.
- **Do not bundle the §Security work.** Host allowlist, CORS,
  random prefix are explicitly out of scope. If a tempting "while
  we're here" arises, write a TODO in the handover and leave it.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's `verify:` list (or `verify_cmd`).
   Every step must pass. On failure: stop, fix, re-run; do not
   advance to `docs`. For Rust stages the baseline is
   `cargo test --workspace`, `cargo clippy --workspace --all-targets
   -- -D warnings`, `cargo fmt --check`. For UI stages add
   `pnpm -C ui/codeless-ui typecheck` and `pnpm -C ui/codeless-ui
   lint`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that opens
   the next stage has the context it needs (per SCOPE Constraint 2:
   anything that must survive a stage boundary is on disk, not in
   the agent's head).
3. `git` — stage the changes (`git add -A` from the worktree root,
   or specific paths if the stage was surgical), commit with the
   message `stage N: <one-line title from template.yaml>` so the
   history mirrors the template stages one-for-one, and push to
   the job's branch (`codeless/workspace-scoping`) so the work is
   recoverable even if the worktree is wiped.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change (e.g. an
investigation stage that only updated `SCOPE.md` and that doc was
already current), say so in the handover and mark `git` as
`skipped — no diff`, but the next stage's commit must include any
side-effect files the investigation touched.
