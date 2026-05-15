# Scope — workspace-attach (server-side, milestones 1 & 2)

The full design is **[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)** in
the workspace repo. This brief is the trimmed per-job scope; the deep
design lives there and wins on any disagreement.

## Goal

Land milestones 1 and 2 of WORKSPACE-ATTACH on `master` via the
`codeless/workspace-attach` branch. After this job, a running server
exposes four typed RPCs (`attach_workspace`, `detach_workspace`,
`list_workspaces`, `validate_workspace_path`), persists the attached
set in SQLite, and serves `fs.*` against a canonical allowed-roots
list. The UI is **not** touched in this job — that's a follow-up.

## In scope

- The four open questions in WORKSPACE-ATTACH.md §"Open questions"
  resolved with reasoning recorded in this file (stage 1).
- `attached_workspaces` SQLite table with canonical + display columns
  and a unique index on the canonical column.
- Idempotent boot upsert from `--fs-root` that canonicalises before
  the index check (trailing slash, `.` segments, symlinks all collapse).
- Wire types in `codeless-types` with `specta` derives:
  `AttachWorkspaceArgs/Result`, `AttachedWorkspace`,
  `DetachWorkspaceArgs`, `DetachPolicy`, `ListWorkspacesResult`,
  `ValidateWorkspacePathArgs/Result`, `WorkspaceProblem`,
  `WorkspaceError`.
- Four RPC methods implemented end-to-end with a round-trip integration
  test (`attach → list → detach`) using the in-memory SQLite harness.
- Host adapter switch from `Option<PathBuf> fs_root` to a canonical
  allowed-roots list; existing `fs.*` calls continue to work and reject
  paths outside the attached set with `PermissionDenied`.
- 30s liveness sweep emitting `workspace_unhealthy` /
  `workspace_recovered` events.
- `ServerInfo.fs_root` frozen to the boot-time `--fs-root` value (does
  not shift as workspaces attach/detach).

## Out of scope

- All UI work. The `/workspaces` page, sidebar group, attach/detach
  modals, and the `PathPicker` shell-injection are milestones 3-6 and
  belong in a follow-up job.
- `worktree-root` becoming per-workspace (open question 2 — bias
  deferred per the doc).
- Drag-and-drop attach in the desktop shell (open question 4 — bias
  deferred).
- Removal of the `--fs-root` flag (open question 1 — bias keep).
- Mobile shell file pickers (Phase 6).
- Anything multi-tenant: bearer token authorises identically (R5).

## Constraints

- **R1** — `tokio::process` / `std::process::Command` may not appear
  in any crate other than `codeless-adapters-host`. The new RPC types
  live in `codeless-types` (iOS-safe, Android-safe); the
  implementations live in `codeless-runtime` and
  `codeless-adapters-host`. Filesystem `stat()` and `git rev-parse` for
  `validate_workspace_path` run in the host adapter only.
- **R4** — `attached_workspaces` rows are the source of truth. No
  in-memory authoritative state; the UI (later) subscribes to events.
- **R5** — every new method sits behind the same bearer gate.
  `validate_workspace_path` is server-side rate-limited (~5/s per
  connection) so a debounced picker can't `stat()`-storm the disk.
- **Comments rule (R2 in codeless/CLAUDE.md)** — no task-status
  comments, no emojis, no restatements. Comments earn their keep only
  for *why*.
- `cargo test --workspace` / `cargo clippy --workspace --all-targets
  -- -D warnings` / `cargo fmt --check` all green before each commit.
- MSRV 1.78.

## Deliverables (what "done" looks like)

1. `codeless/workspace-attach` branch with one commit per stage,
   pushed via mani.
2. `cargo test --workspace` green; the round-trip
   `attach → list → detach` test exists in `codeless-runtime` and
   exercises an in-memory SQLite pool plus the host adapter.
3. The canonicalisation test asserts `/a/b`, `/a/b/`, and a symlink
   pointing at `/a/b` all upsert into a single row.
4. `ServerInfo.fs_root` returns the boot-time value (or `None`);
   `list_workspaces` returns the live set.
5. `WORKSPACE-ATTACH.md` §"Open questions" is updated in the workspace
   repo with the chosen answers and one-line rationales.

## Open questions (resolve in stage 1, before any code)

These are the four questions in WORKSPACE-ATTACH.md §"Open questions".
Record the chosen answer + one-line *why* directly under each in this
file, then update WORKSPACE-ATTACH.md to match.

1. Remove `--fs-root` or keep it as a bootstrap convenience? (Bias:
   keep.)
2. Should `worktree-root` become per-workspace? (Bias: defer; flag the
   coupling with `DetachPolicy::LeaveRunning`.)
3. Should detach archive the repo row or leave it registered-but-
   detached? (Bias: leave; `remove_repo` is the destructive verb.)
4. Drag-and-drop folder attach on desktop in milestone 1? (Bias: no.)

Do not silently re-bias. If a decision diverges from the doc's bias,
explain *why* in this file and update WORKSPACE-ATTACH.md.

## References

- Workspace doc (authoritative): [`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
- Project scope: [`DOCS/SCOPE.md`](../../../DOCS/SCOPE.md)
- Agent rules (codeless repo): [`CLAUDE.md`](../../../codeless/CLAUDE.md)
- Agent rules (workspace): [`CLAUDE.md`](../../../CLAUDE.md)
- Crate layout & R1: [`DOCS/SCOPE.md#crate-layout-load-bearing-not-aspirational`](../../../DOCS/SCOPE.md#crate-layout-load-bearing-not-aspirational)
