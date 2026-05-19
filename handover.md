# workspace-scoping — stage 4 → done

Stage 4 threaded `repo_id` through every `fs.*` RPC and replaced the
"single global fs_root" jail with a per-call jail looked up from
`attached_workspaces`. Stage 5 is the REVIEW gate that locks the
server-side scoping (subscribe + fs.*) before the UI plumbing starts.

## Wire shape changes (load-bearing for stage 5)

- `FsReadDirArgs`, `FsReadFileArgs`, `FsWriteFileArgs`, `FsStatArgs`,
  `FsCreateFileArgs`, `FsCreateDirArgs`, `FsMoveArgs`, `FsDeleteArgs`
  all gained a required `repo_id: RepoId` field.
- New `FsCwdArgs { repo_id: RepoId }`. `fs_cwd` no longer takes a
  unit body — the trait signature is
  `fs_cwd(&self, args: FsCwdArgs) -> RpcResult<FsCwdResult>`.
  `FsCwdResult` shape is unchanged.
- `wire-rpc.ts.snap` regenerated; the generated TS bundle at
  `ui/codeless-ui/src/lib/rpc/generated/wire.ts` is in sync.
- `crates/codeless-rpc/examples/wire_ts.rs` and
  `crates/codeless-rpc/tests/specta_snapshot.rs` now register
  `FsCwdArgs`, `FsCreateFileArgs`, `FsCreateDirArgs`, `FsMoveArgs`,
  `FsDeleteArgs` — these had been missing from the wire registry
  even though `methods.rs` defined them, so the UI maintained
  hand-rolled copies in `lib/rpc/methods.ts`. Stage 6 picks the
  generated shapes back up.

## Runtime semantics

- `InProcessRpc::fs_root_for_repo(repo_id) -> RpcResult<PathBuf>` is
  the canonical lookup. Reads `attached_workspaces.fs_root_canonical`
  and verifies the path is currently in `HostFs::roots()`; both
  "unknown `repo_id`" and "row exists but adapter no longer trusts
  the root" return `RpcError::NotFound("workspace not attached: …")`
  so the UI's empty-state copy can branch on a single error.
- Every `fs.*` handler in `crates/codeless-runtime/src/rpc/fs.rs`
  resolves the jail through this helper, then dispatches to the new
  `HostFs::*_in(jail, ...)` family (see below). The old
  `HostFs::resolve()` / `read_dir()` / `read_file()` / … methods are
  retained — `agent_chat` and the worktree manager still use them
  via the legacy "any registered root" path.
- Detached workspaces fail before the adapter is consulted:
  `fs_calls_against_detached_repo_id_are_refused` pins that.

## Host adapter additions

`crates/codeless-adapters-host/src/fs.rs`:

- `HostFs::root_is_registered(root: &Path) -> bool` — canonical-aware
  membership check used by `fs_root_for_repo` for defence-in-depth.
- `HostFs::resolve_in(jail, path)` (private) — same `ParentDir`
  refusal and `canonicalize + starts_with(jail)` check as the
  legacy `resolve`, but scoped to one jail. Missing-tail path
  (file-to-create) handling preserved.
- `HostFs::read_dir_in`, `read_file_in`, `write_file_in`, `stat_in`,
  `create_file_in`, `create_dir_in`, `rename_in`, `delete_in` — eight
  new public methods, each `(jail: &Path, ...)`. The shared
  `read_dir_at` / `stat_at` helpers were factored out so the old
  and new paths share one body.

## Tests added / updated

- `crates/codeless-runtime/tests/fs.rs` rewritten to attach a
  workspace via `add_repo` + `attach_workspace` instead of pointing
  `HostFs::new` at a tempdir directly. New cases:
  - `fs_unknown_repo_returns_not_found`
  - `fs_read_dir_is_jailed_to_the_repo_id` (the cross-tab leakage
    regression: workspaces A and B attached to the same `HostFs`;
    a call with A's `repo_id` cannot see B's files even via an
    absolute path)
  - `fs_cwd_returns_the_repos_canonical_root`
  - `fs_root_for_repo_helper_matches_attached_workspaces`
  - `fs_calls_against_detached_repo_id_are_refused`
- `crates/codeless-client/tests/round_trip.rs` and
  `crates/codeless-server/tests/routes.rs` updated to attach a
  workspace before exercising the HTTP `fs.*` wire.

## Verify

- `cargo test --workspace --no-fail-fast` — 71 suites green.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.

## What stage 5 (REVIEW) is voting on

The wire surface stage 6 will bind to:

- `EventFilter::{All, Job{job_id}, Repo{repo_id}, Library}` (locked
  in stage 3).
- `fs.*` RPCs all take `repo_id: RepoId` (locked here).
- `fs_cwd` takes `FsCwdArgs { repo_id }` rather than a unit body.
- `RpcError::NotFound` is the typed refusal for unknown / detached
  `repo_id` across the whole `fs.*` surface — same shape as today's
  "unknown job" / "unknown review" so the UI's existing 404 handler
  picks it up.

If the gate passes, stage 6 starts. If the gate rejects the shape,
the diff is contained to the 16 files this stage touched plus their
snapshot regeneration.

## Known follow-ups (not in this stage)

- `methods.ts` in `ui/codeless-ui/src/lib/rpc/` still hand-rolls
  `FsReadDirArgs` etc. without `repo_id`. Stage 6 either deletes
  those duplicates in favour of the generated `wire.ts` or threads
  `repo_id` through the manual copies — flagged here so the next
  agent doesn't reintroduce drift.
- Every UI call site (`App.tsx`'s `fs_cwd` bootstrap, `useFileTree`,
  `useDocument`, `HandoverPanel`, `composer.tsx`, `native.ts`,
  `RunPane.tsx`, `CwdBreadcrumb.tsx`, `NewEditorDialog.tsx`) needs
  `repo_id` passed. Stage 6's job.
- `attach_workspace` mirrors into `HostFs::add_root`; the legacy
  `HostFs::root()` (first-registered) still backs `agent_chat`'s
  cwd resolution. That code path is intentionally untouched in
  this stage.
