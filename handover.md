# workspace-scoping — stage 6 → done

Stage 6 plumbed `activeRepoId` through every UI `fs.*` call and
flipped the workspace-picker's subscription to the new
`{ scope: "library" }` channel. The file explorer now rehydrates
when the active workspace changes because `useFileTree` keys on
`repoId` and clears its cached nodes/expanded state when it flips.

## Wire bindings tightened to the generated shapes

`ui/codeless-ui/src/lib/rpc/methods.ts` no longer hand-mirrors the
`fs.*` arg structs. The eight FS arg/result types that the server
emits today (`FsCreateDirArgs`, `FsCreateFileArgs`, `FsCwdArgs`,
`FsCwdResult`, `FsDeleteArgs`, `FsMoveArgs`, `FsReadDirArgs`,
`FsReadDirResult`, `FsReadFileArgs`, `FsWriteFileArgs`) are now
re-exported from `./wire` so the `repo_id: RepoId` field that landed
in stage 4 is part of `RpcArgs<"fs_*">` and TypeScript refuses to let
a call site forget it. `fs_cwd`'s `RpcMethodMap` entry switched from
`Record<string, never>` to `FsCwdArgs`. `fs_search` and `fs_glob`
remain hand-mirrored (they're mock-only — no server-side
implementation) but they gained `repo_id` for parity so call sites
don't have to branch on which transport they're targeting.

Side effects of switching to the generated shapes:

- `FsReadFileArgs` no longer carries `byte_limit`. Three call sites
  (`HandoverPanel`, `useDocument`, `native.readFile`, the composer's
  `attachFileByPath`) lost the `byte_limit: null` field; the mock
  client now uses `MOCK_FS_READ_LIMIT_DEFAULT` unconditionally.
- `FsWriteFileArgs` no longer carries `create_parents`. `useDocument`
  and `native.writeFile` dropped the field; the mock no longer
  auto-creates parent directories on write (matches server behaviour).

## Call-site updates

Every `fs.*` invocation outside of tests/mocks now passes `repo_id`:

- `App.tsx` — `fs_cwd` bootstrap reads `useWorkspacesStore` for the
  active id and only fires once a workspace is attached. The shell
  `paths.homeDir()` still wins where the shell exposes one (desktop,
  iOS, Android); browser/mobile-PWA hits the RPC fallback.
- `useFileTree` — gained a required `repoId: RepoId | null` param.
  When `null`, the tree refuses to fetch and renders empty. `repoId`
  is in the deps of the bootstrap effect, so flipping it resets
  `nodes`/`expanded`/`pendingCreate`/`renaming`. `FileExplorer` now
  takes (and threads) the same prop; `App.tsx` passes
  `activeRepoId` through.
- `ExplorerSearch` — gained a `repoId` prop; debounced `fs_glob`
  short-circuits when `repoId` is null.
- `useDocument` — pulls `activeRepoId` from the store; `fs_read_file`
  and `fs_write_file` both pass it. A missing workspace surfaces as
  the editor's `error` state rather than throwing through React.
- `NewEditorDialog` — pulls `activeRepoId`, refuses to create when
  it's null.
- `HandoverPanel`, `RunPane`'s `fs_stat` probe — read `repo_id` off
  the `Job` row they already hold (`job.repo_id`). This is the
  correct identity because the job's worktree was created against
  that workspace; reading it via the active picker would silently
  break when the user switches workspaces while a job page is open.
- `CwdBreadcrumb::CurrentSegmentDropdown` — `fs_read_dir` pulls
  `activeRepoId` from the store.
- `native.ts` (ai-tools surface) — added `activeRepoIdOrThrow()` that
  reads `useWorkspacesStore.getState().activeRepoId` at *call* time
  (not configure time), so a workspace switch mid-tool-sequence
  retargets the next call without re-binding the singleton. All
  `fs.*` helpers (`readFile`, `writeFile`, `createFile`, `createDir`,
  `readDir`, `grep`, `glob`) now thread it through.
- The composer's `attachFileByPath` reads the store at call time too.

## Library-scope picker subscription

`useWorkspacesSync` flipped from `{ scope: "all" }` to
`{ scope: "library" }`. The `Library` variant landed in stage 3 — see
[`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
§"RPC additions" for the contract. The picker now receives only
workspace-lifecycle envelopes (`workspace-attached`,
`workspace-detached`, `workspace-unhealthy`, `workspace-recovered`)
and never the per-job firehose, so two browser tabs viewing two
different workspaces can both keep their picker live without leaking
job events.

The mock client's `subscribe` now switches on all four scope variants
(`all` / `job` / `repo` / `library`). `library` filters to envelopes
without a `job_id`, which is the closest the mock can come to the
runtime's `repo-only` filter — mock envelopes don't yet carry
`repo_id`, so the `repo` variant uses an `as unknown as { repo_id? }`
peek; a future stage that emits real workspace-scoped events from the
mock will tighten that. Two existing test doubles
(`ChatMessageList.test.tsx`, `ChatTab.test.tsx`) also gained the
narrower discriminant check so TypeScript narrows `filter.job_id`
correctly on the `job` arm.

## Other `{ scope: "all" }` callers stay as-is

`JobsDashboard`, `AssistantPage`, `AssistantFooterBar`, and
`RunPane`'s waitForCompletion still use `{ scope: "all" }` or
`{ scope: "job" }` — the latter is correct as-is. The former three
are the global event-log views that the SCOPE.md explicitly leaves
alone for this job; they don't drive workspace-scoped state and they
all live on pages that span workspaces. Stage 7 or a follow-up may
revisit `JobsDashboard` once the dashboard learns to filter to the
active workspace.

## Verify

- `pnpm -C ui/codeless-ui typecheck` — clean.
- `pnpm -C ui/codeless-ui test` — 135 passed, 27 files.
- `pnpm -C ui/codeless-ui lint` — no eslint configured (no-op).
- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace --no-fail-fast` — green. One flaky
  `git_diff::tests::missing_base_ref_is_distinct_error` on a single
  hot run; passes in isolation and on rerun. Unrelated to this stage.

## What stage 7 picks up

- Deep-link router: read `?workspace=<repo_id>` on load, write it on
  every `setActive` via `history.replaceState`. Stage 6 wired the
  store's `setActive` so refresh-survival becomes a router change,
  not a store change.
- Tauri webview smoke: stage 6's changes ride the shared UI tree, so
  `TauriIpcClient` callers see the new `fs.*` shape too. Run a
  `--native-window` boot at the end of stage 7 to confirm the
  desktop shell still loads against the same wire.
- A small drift risk lives in `wire.ts` (the manual file) vs
  `generated/wire.ts`: the manual file's `FsReadResult` still has the
  binary / toolarge variants the server doesn't emit. Stage 6 left
  it alone because the editor's state machine still branches on the
  kinds, but a future stage either deletes the manual file or
  promotes its variants into specta.

## Known follow-ups (not in this stage)

- `JobsDashboard` still uses `{ scope: "all" }`; per-workspace
  filtering of the jobs list is a stage 7+ decision.
- The mock client's `subscribe` `repo` branch peeks at `repo_id` via
  a cast because mock envelopes don't carry one. When stage 7's
  router tests need workspace-scoped mock events, tighten this.
- `methods.ts`'s opening comment still mentions stage-15 of the UI
  conversion loop; the `fs.*` types are now generated, so a future
  cleanup pass can shorten the preamble.
