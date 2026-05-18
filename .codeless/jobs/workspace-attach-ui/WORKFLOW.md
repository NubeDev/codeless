# Workflow — workspace-attach-ui

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the workspace
[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md).

## Sequencing

- Stage 1 (M3a) is the foundation — the `RpcClient` interface change
  forces typecheck failures across the codebase the moment any
  consumer is added, so it lands first and alone.
- Stages 2-3 (M3b, M3c) may be done in either order but must not be
  batched; each ships its own commit so the diff is a coherent unit
  and a revert is one commit.
- Stage 4 is a REVIEW gate. M3 must be approved before any M4
  (visible UI) work begins.
- Stages 5-7 (M4a, M4b, M4c) build on each other linearly. M4a
  (store + events + empty state) must be live before M4b (the table
  + attach modal) so the table renders against the real store. M4c
  (detach modal + Playwright tests) closes the loop.
- Stage 8 is the final REVIEW gate.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in §"In scope", **stop and surface it** in
   the job chat — do not silently expand scope.
2. Re-read the relevant section of `DOCS/WORKSPACE-ATTACH.md`. The
   workspace doc is authoritative; this job's `SCOPE.md` is the
   brief.
3. Re-read [`DOCS/UI-ARCHITECTURE.md`](../../../DOCS/UI-ARCHITECTURE.md)
   §"RpcClient boundary" and §"Shell-injected capabilities" so the
   new code matches the existing pattern; do not invent a new
   injection mechanism.
4. Check R2 / R3 before committing. Grep:
   ```
   rg '@tauri-apps' ui/codeless-ui/src --glob '!src/shells/desktop/**'
   ```
   The match set must not grow. R3 forbids `.web.tsx` /
   `.desktop.tsx` forks; grep:
   ```
   rg --files ui/codeless-ui/src | rg '\.(web|desktop|android|ios)\.tsx?$'
   ```
   The match set must stay empty.

Before committing a stage:

1. `pnpm -C ui/codeless-ui lint` green.
2. `pnpm -C ui/codeless-ui test` green (Vitest + RTL).
3. For M4 stages: the Playwright happy-path test for the modal
   landed in that stage actually exercises the new behaviour
   (clicks through to a server-confirmed attach / detach against
   the mock client), not just renders the markup.
4. The stage's snapshot tests, if any, are updated intentionally
   (review every diff line — no blind `-u`).
5. Update `SCOPE.md` §"Deliverables" with a `[x]` against anything
   completed in the stage.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## REVIEW gates

Two gates: stage 4 (M3 complete) and stage 8 (M4 complete).

At each gate, write a handover comment in the job chat with:

- One bullet per item the gate is checking.
- For stage 4: confirm all four methods are callable from both
  `HttpSseClient` and `TauriIpcClient`, the typed-wire snapshot
  passes, and the `PathPicker` injection works in dev (`pnpm dev`
  in browser shell, `pnpm tauri dev` in desktop shell). Paste the
  manual-smoke transcript proving a `listWorkspaces()` call returns
  the seeded boot row.
- For stage 8: paste `pnpm -C ui/codeless-ui test` tail, the
  Playwright report summary, and a screenshot (or markdown render)
  of the Settings → Workspaces tab in three states: empty, one
  attached, one attached + warning badge mocked.

Do not proceed past a REVIEW gate without explicit approval in chat.

## Anti-patterns specific to this job

- **Do not** import `@tauri-apps/*` outside
  `ui/codeless-ui/src/shells/desktop/`. The browser shell injector
  for `PathPicker` uses the Web `showDirectoryPicker` API only;
  there is no Tauri import path in any non-desktop file.
- **Do not** create a `WorkspacesPage.web.tsx` /
  `WorkspacesPage.desktop.tsx` split. R3 is non-negotiable; the
  picker is the only difference and it goes through the
  shell-injected interface.
- **Do not** call `attach_workspace` without first calling
  `validate_workspace_path`. Even though the server validates again,
  the picker must give live feedback — never let the user click
  Attach against an invalid path and find out only on submit.
- **Do not** string-match `WorkspaceError`. The doc introduces
  structured variants (`AlreadyAttached`, `RunningJobs`,
  `PathRejected`, `NotAttached`) precisely so the UI renders typed
  messages. Use the discriminator.
- **Do not** treat `Conflict` as a generic error in the attach path.
  The unique index on `attached_workspaces.fs_root_canonical` makes
  the second simultaneous attach a `Conflict` that should render as
  "already attached" (per doc §Edge cases), not as a red toast.
- **Do not** start M5/M6/M7 work because a file is "almost there".
  Scope creep on a UI grind is how this lands at 5x cost. If a
  follow-up TODO surfaces, write it in a stage-end handover note,
  do not implement.
- **Do not** start the `/workspaces` top-level route or the sidebar
  group. They are explicitly deferred to the follow-up job.
- **Do not** drift the `PathPicker` interface signature. The doc
  pins it to `pickDirectory(opts?: { startPath?: string }):
  Promise<string | null>`; any extension is a separate decision and
  belongs in the open-questions discussion, not in code.
- **Do not** make the attach modal create a `Repo` row implicitly
  if one already exists for the canonical path. The decision
  recorded in stage 1 (SCOPE §"Open questions" Q1) is to look up
  first; only call `add_repo` when the lookup misses.

## When to halt

- A typed-wire snapshot mismatch you cannot explain: stop, do not
  regenerate the snapshot, surface in chat. Snapshot drift on the
  RPC boundary is the canary for an upstream type change you missed.
- `pnpm -C ui/codeless-ui test` fails after a real fix attempt and
  the next move is not obvious: mark the stage `[!]` in `SCOPE.md`
  and stop. Do not commit a partial implementation with a TODO
  (R4 in codeless/CLAUDE.md).
- Any R2 / R3 grep regression (new `@tauri-apps/*` outside
  `src/shells/desktop/`, or a new `*.{web,desktop,android,ios}.tsx`
  file): halt and rework. Both are non-negotiable.
- A stage's work needs a decision not in stage 1's resolved list:
  stop, surface the decision in chat, do not silently choose.
