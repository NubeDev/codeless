# Build status — Phase 2 UI RPC conversion (fs.* / secrets.* surface)

> ⛔ AGENT REMINDER — READ BEFORE TOUCHING THIS FILE
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: workspace `CLAUDE.md` and
>    `codeless/CLAUDE.md`.
> 2. One logical batch per tick. Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3.
> 3. You MUST schedule the next tick before exiting — `CronCreate`
>    recurring:false ~1 min out. If all stages `[x]`, report DONE.
> 4. Update this file in the same commit as the code change.
> 5. ⛔ COMMIT AND PUSH via mani every stage. Never `--force`, never
>    `--no-verify`. If push fails, mark `[!]` and halt.
> 6. ⛔ Comments explain *why*. No emojis. No task-status comments.
> 7. ⛔ R2 transport boundary is enforceable. No new `@tauri-apps/*`
>    imports outside `src/shells/<shell>/`. Documented exceptions:
>    `src/lib/rpc/tauri-ipc-client.ts`, `src/settings/main.tsx`,
>    and the two PTY files (out of scope for this loop).
> 8. ⛔ Do NOT edit `codeless/crates/*`. UI-only loop. Halt if a
>    stage seems to need a Rust change.

File: DOCS/sessions/2026-05-12-phase-2-ui-rpc-conversion.md
Goal: Drive `@tauri-apps/*` import count from 13 → ≤3 (the two
      documented exceptions plus the two PTY files) by extending
      the RPC surface with `fs.*` / `secrets.*` method types,
      implementing them in `MockRpcClient`, then converting the
      10 blocked UI files to call `useRpc()`. Verified end-to-end
      against `MockRpcClient` at `/?mock=1`.
Started: 2026-05-12
Last tick: 2026-05-12 (stages 13 + 14 — target hit, 13→4 files)
Current stage: 15 ← next (specta codegen, gated on Rust snapshots)

Repo:        codeless
Branch:      feat/phase-2a-persistence  (Phase 2 UI work stacks on
             the same branch as Phase 2a/2b/2c per kickoff
             override "KEEP SAME BRANCH")
Memory policy: compact every 3 stages
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

Scope guardrails:
  - Touches only `codeless/ui/codeless-ui/`. Zero edits in
    `codeless/crates/*`.
  - `fs.*` / `secrets.*` shapes are hand-mirrored from the
    forthcoming Rust additions; stage 15 replaces with specta
    snapshots.
  - Do NOT touch `pty-bridge.ts`, `useTerminalSession.ts`, or
    `settings/main.tsx`.

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag mandatory.

- [x] 1.  [S] `src/lib/rpc/wire.ts` — FS wire types (`FsEntry`,
          `FsReadResult` union, `FsGrepHit`, `FsGlobHit`).
- [x] 2.  [S] `src/lib/rpc/methods.ts` — `fs_*` method types
          (read_file, write_file, create_file, create_dir,
          read_dir, search, glob, move, delete, cwd).
- [x] 3.  [S] `methods.ts` — `secrets_{set,get,list,rm}` types.
- [x] 4a. [M] `MockRpcClient` — in-memory FS rooted at
          `/home/user/mock-workspace` (seed fixture: README.md,
          src/index.ts, docs/notes.md), in-memory secrets map,
          ~80ms latency, `not_found` / `conflict` /
          `invalid_argument` error kinds. (Note: wire error type
          has no `permission_denied` variant — using
          `invalid_argument` for type-mismatch cases; no
          permission-denied path in mock since there is no real
          permission model.)
- [ ] 4b. [S] Vitest bootstrap + happy-path spec for FS read/
          write/list/delete + secrets set/get/list/rm. Deferred:
          UI package has no vitest dep yet; bootstrap is its own
          stage. Stages 5–12 verify against `/?mock=1` instead.
- [x] 5.  [S] Convert `modules/editor/lib/useDocument.ts`
          (fs_read_file, fs_write_file).
- [x] 6.  [S] Convert `modules/editor/NewEditorDialog.tsx`
          (fs_create_file).
- [x] 7.  [S] Convert `modules/explorer/lib/useFileTree.ts`
          (fs_read_dir, fs_create_dir, fs_create_file, fs_move,
          fs_delete — absorbed stage 8 territory since the rename/
          delete handlers live here, not in contextActions.ts).
- [x] 8.  [S] Convert `modules/explorer/lib/contextActions.ts` —
          revealItemInDir moved to a new `revealPath` method on
          `ExternalOpenerAdapter` (browser no-op; Tauri shell uses
          `@tauri-apps/plugin-opener`). Callers updated.
- [x] 9.  [S] Convert `modules/explorer/ExplorerSearch.tsx` —
          path-search wired to `fs_glob` with `**{q}**` pattern
          (the old `fs_search` Tauri command had path-shaped
          results; fs_glob is the right primitive). Content
          search remains future work.
- [x] 10. [S] Convert `modules/statusbar/CwdBreadcrumb.tsx` —
          the dropdown's `list_subdirs` Tauri command replaced
          with `fs_read_dir` + filter to dir entries. (No
          `fs_cwd` call lives here — cwd is passed in as a prop.)
- [x] 11. [S] Convert `modules/ai/lib/keyring.ts` to secrets_*.
          Functions now take `RpcClient` as first arg; three
          callers updated (App.tsx, EditorPane.tsx,
          ModelsSection.tsx). Dropped `getAllKeys`'s batch
          `secrets_get_all` fallback — wire surface has no batch
          method.
- [x] 12. [S] Convert `modules/ai/lib/composer.tsx`
          `attachFileByPath` (fs_read_file).
- [x] 13. [M] Convert `modules/ai/lib/native.ts` — done after
          extending the RPC surface with `shell_run`,
          `shell_session_{open,run,close}`, and
          `shell_bg_{spawn,logs,kill,list}` (wire + methods +
          mock). `native.ts` kept its free-function shape because
          its callers (`ai/store/planStore.ts`, four
          `ai/tools/*.ts` files, `AgentRunBridge.tsx`,
          `ai/lib/transport.ts`) live outside React's tree. A
          new `configureNative(rpc)` setter is called once from
          `App.tsx`; all `invoke()` paths become `rpc().call()`.
          The shell mock returns canned `[mock] <cmd>` output —
          real execution stays on the Rust side per R1.
          NOTE: the PTY streaming channel is intentionally NOT
          part of this surface; it remains the SCOPE.md-reserved
          WebSocket transport and lives behind `pty-bridge.ts` /
          `useTerminalSession.ts` (kept out of scope here).
- [x] 14. [S] Audit pass — final count 4 (target ≤4 hit). Updated
          `DOCS/UI-PORT-AUDIT.md`. The 4 residuals are
          `tauri-ipc-client.ts`, `settings/main.tsx` (documented
          exceptions), plus `pty-bridge.ts` and
          `useTerminalSession.ts` (PTY transport, separate work).
- [ ] 15. [S] Specta codegen wiring — DEFERRED. The Rust track
          owns this: it must add `fs_*` / `secrets_*` / `shell_*`
          types to `codeless-types` and regenerate the
          `wire.ts.snap`. Once that snapshot exists, replace
          the hand-mirrored sections of `wire.ts` and (when the
          methods snapshot lands) `methods.ts`, then add a CI
          script that runs `git diff --exit-code` against the
          regenerated TS. Cannot land this in a UI-only loop.

## Notes
- Branch override: kickoff specified `feat/phase-2-ui-rpc-conversion`
  but the operator override `KEEP SAME BRANCH` keeps work on
  `feat/phase-2a-persistence` (stacks on Phase 2a/2b/2c).
- Pre-flight commit: the shell-injection capability adapters
  (10 `src/lib/shell/*` + 10 `src/shells/desktop/*` files) were
  uncommitted at tick 0; landed as commit `phase 2: shell-injection
  capability adapters (browser/desktop split)`.
- Coordination with parallel Rust session: mani's `git add -A` swept
  unrelated crate files into one commit early on; remaining stages
  used explicit pathspec (`git -C codeless add ui/codeless-ui/...`).
  Operator confirmed: "leave the rust/backend code alone".
- Final import count: 13 → 5. The 5 residuals are
  `tauri-ipc-client.ts` and `settings/main.tsx` (documented
  exceptions), `pty-bridge.ts` and `useTerminalSession.ts` (PTY,
  out of scope), and `native.ts` (stage 13 halt — shell surface).
- Stages 8–12 also pulled in incidental edits outside the original
  per-stage file list (FileTreeNode.tsx, FileExplorer.tsx,
  EditorPane.tsx, App.tsx, ModelsSection.tsx) to thread RpcClient
  / opener adapter through to free-function modules. tsc clean
  after every batch.
