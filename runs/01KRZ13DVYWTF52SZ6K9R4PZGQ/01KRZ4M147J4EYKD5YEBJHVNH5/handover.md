## Done

- Replaced hand-mirrored fs.* arg types in `ui/codeless-ui/src/lib/rpc/methods.ts` with re-exports of the generated wire types so `repo_id: RepoId` is part of `RpcArgs<"fs_*">`. Switched `fs_cwd` RpcMethodMap entry from `Record<string, never>` to `FsCwdArgs`. `fs_search`/`fs_glob` stay hand-mirrored (mock-only) but gained `repo_id` for parity.
- Threaded `repo_id` through every UI fs.* call site: `App.tsx` fs_cwd, `useFileTree` (new required `repoId` prop), `FileExplorer`, `ExplorerSearch`, `useDocument`, `NewEditorDialog`, `HandoverPanel`, `RunPane.fs_stat`, `CwdBreadcrumb`, composer's `attachFileByPath`, and the `native.ts` agent-tool surface (via `activeRepoIdOrThrow()` read at call time). Job-owning components use `job.repo_id`; workspace-owning views read `useWorkspacesStore.activeRepoId`.
- `useFileTree` clears cached nodes/expanded/pending state when `repoId` flips, so the file explorer rehydrates on a workspace switch.
- `useWorkspacesSync` flipped from `{ scope: "all" }` to `{ scope: "library" }` so the picker has its own workspace-lifecycle-only channel.
- Mock client `subscribe` narrows on all four EventFilter variants (`all`/`job`/`repo`/`library`); two test doubles (`ChatMessageList.test.tsx`, `ChatTab.test.tsx`) widened their narrowing.
- Side effects from picking up the generated shapes: dropped `byte_limit` from fs_read_file callers and `create_parents` from fs_write_file callers; mock client uses default read limit unconditionally.
- Verify: `pnpm -C ui/codeless-ui typecheck` clean, `pnpm test` 135/135, `cargo fmt --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace --no-fail-fast` green (one transient `git_diff::tests::missing_base_ref_is_distinct_error` flake, passes in isolation, unrelated to this stage).
- Committed `dd0819b` and pushed to `codeless/workspace-scoping`.

## Next

- Stage 7: deep-link router. Read `?workspace=<repo_id>` on load; write it on every `setActive` via `history.replaceState`. Smoke the `--native-window` Tauri webview path at the end of the stage to confirm the desktop shell still loads against the new fs.* wire.

## What you need to know

- `mani` is not present in this isolated worktree, so the commit + push used raw `git`. The job-loop instructions in `CLAUDE.md` say to prefer `mani`; if the loop wrapper requires that path, re-do the commit through mani from the workspace root on the next stage.
- `methods.ts` re-exports its fs.* types from `./wire` (the generated module), not the manual `wire.ts`. `FsReadResult` (manual `wire.ts`) still carries `binary` / `toolarge` variants that the server doesn't emit — the editor's state machine still branches on those kinds, so the manual file was left alone. Drift between the two `wire*.ts` files is worth a follow-up but is out of scope here.
- `JobsDashboard`, `AssistantPage`, `AssistantFooterBar` still use `{ scope: "all" }`. SCOPE.md leaves them for a later stage; they're the global event-log surfaces. Don't widen scope-flipping into them under stage 7.
- `native.ts` reads `activeRepoId` from the store at call time (not boot time). A workspace switch mid-tool-sequence retargets the next call; tools that captured a path against the old workspace will fail with the typed `NotFound`, which is the desired behaviour.
- `useFileTree`'s `repoId` prop is required (non-optional) — any new caller must pass it.

## Open questions

- The mock `subscribe` `repo` branch peeks at `repo_id` via an `as unknown as { repo_id? }` cast because mock envelopes don't yet carry one. Stage 7's router tests may need real mock workspace events; tighten when that happens.
- `methods.ts`'s preamble still references "stage 15 of the UI conversion loop"; the fs.* types are now generated, so a future cleanup pass can shorten it. Did not touch it here to keep the diff focused.
