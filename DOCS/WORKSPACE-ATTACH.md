# WORKSPACE-ATTACH — Scope

Status: server-side complete (M1, M2); UI not started (M3-M6)
Owner: ap@nube-io.com
Created: 2026-05-15
Last status update: 2026-05-15

## Summary

Today the codeless server is told *where to work* by a CLI flag
(`--fs-root <path>`) at boot. To switch to a different repo on disk,
the operator stops the server, edits the flag, and restarts. That's
fine for one-off demos and impossible for the actual product.

This document specifies an **in-app "Workspaces" surface** that lets
the user **attach** and **detach** repos at runtime, from both the
browser and the Tauri desktop shell, with no server restart. The
server gains a small typed RPC for managing the set of attached
workspaces and persists the set in SQLite (R4). The UI is one
responsive component shipped to all four shells (R3).

> **Sister docs.** [`SCOPE.md`](./SCOPE.md) §"Workspaces" defines
> what a workspace *is*. [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md)
> defines the `RpcClient` boundary the UI must stay behind. Where
> this doc disagrees with either, **those win** — open an issue and
> update this file.

## Goals

1. The user starts the server **once**, with no `--fs-root`, then
   uses the UI to point it at one or more repos.
2. **Attach**: pick a directory on disk, name it, optionally pick the
   default branch + runner, click *Attach*. The server validates the
   path, registers a repo, opens the `fs.*` surface for that root,
   and persists the row.
3. **Detach**: pick an attached workspace, click *Detach*. Running
   jobs against it are surfaced; the user must stop them or move them
   to "orphaned" before the row disappears.
4. **Switch**: a sidebar entry per attached workspace; clicking one
   makes it the *active* workspace (file tree, editor, jobs filter
   all reflect it). Active state is per-tab (UI), not per-server.
5. Same UX in the **browser** and **Tauri desktop** shells. The path
   picker is the only thing that legitimately differs — implement as
   a shell-injected interface (R3).
6. No CLI restart for any of the above. `--fs-root` becomes a
   *bootstrap convenience*, not the source of truth.

## Non-goals

- Multi-server federation. One core, many workspaces — not many cores.
- Automatic discovery / scanning. The user names their workspaces
  explicitly.
- Per-workspace bearer tokens. Single trust boundary (R5).
- Mobile-shell file pickers. iOS / Android (Phase 6) defer to
  attach-via-clone-URL only; on-device local paths are out of scope.
- Multi-tenant isolation. Single user, single trust boundary (R5).
- Renaming the underlying directory on disk. Display name only;
  `local_path` is immutable for the lifetime of the row.

## Concepts

A **workspace** is the runtime tuple `(repo row, fs_root path,
worktree subdir)`. The repo row already exists (`add_repo`); this
doc adds the **attach lifecycle** around it:

```
| State      | Repo row | fs.* RPC for this root | Jobs allowed |
|------------|----------|------------------------|--------------|
| detached   | yes      | no                     | no (refused) |
| attached   | yes      | yes                    | yes          |
```

The server has exactly two states per repo: detached and attached.
**`active` is a UI projection only** — every browser tab / desktop
window picks one attached workspace as its current view. The server
doesn't know which one is active and serves all attached roots
equally.

## RPC additions

Today's flat `fs_root` (one option string in `ServerInfo`) becomes a
**list of attached roots**, managed via four typed methods. The
existing `add_repo` / `remove_repo` stay; attach/detach is a
separate verb because a repo can exist without being attached
(useful for "I want to register it but not let the editor in yet").

```rust
// codeless-rpc / methods.rs

#[derive(Serialize, Deserialize, specta::Type)]
pub struct AttachWorkspaceArgs {
    pub repo_id: RepoId,
    /// Override the repo's `local_path`. Only used to resolve symlinks
    /// or pick a sub-tree. The canonicalised override must be a
    /// descendant of the canonicalised `local_path`, and dotfile
    /// directories like `.git` are rejected. When set, the override
    /// becomes the `fs.*` jail for this workspace — `fs.*` calls
    /// outside it return `PermissionDenied`, even if they are still
    /// inside `local_path`.
    pub fs_root_override: Option<String>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct AttachWorkspaceResult {
    pub workspace: AttachedWorkspace,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct AttachedWorkspace {
    pub repo_id: RepoId,
    pub repo_name: String,
    pub fs_root: String,           // canonical absolute path
    pub attached_at: UnixMillis,
    pub default_runner: Option<RunnerId>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ListWorkspacesResult {
    pub workspaces: Vec<AttachedWorkspace>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct DetachWorkspaceArgs {
    pub repo_id: RepoId,
    /// `Stop` stops every running job against this workspace before
    /// detaching. `LeaveRunning` detaches the editor surface but lets
    /// jobs keep running in their worktree (they retain a
    /// runner-scoped `fs.*` handle; the *editor* loses access).
    /// `Refuse` is the default — if there are running jobs, the call
    /// returns `RunningJobs { jobs: Vec<JobId> }` and detaches
    /// nothing.
    pub on_running_jobs: DetachPolicy,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub enum DetachPolicy { Refuse, Stop, LeaveRunning }

/// Structured error variants used by attach/detach so the UI does
/// not have to string-match on a generic `Conflict`.
#[derive(Serialize, Deserialize, specta::Type)]
pub enum WorkspaceError {
    AlreadyAttached { repo_id: RepoId, fs_root: String },
    RunningJobs    { jobs: Vec<JobId> },
    PathRejected   { problems: Vec<WorkspaceProblem> },
    NotAttached,
}
```

Method routes (under `/rpc/`, behind the bearer gate where present):

```
| Method                  | Verb effect |
|-------------------------|-------------|
| attach_workspace        | mark a repo as attached, allow fs.* under its root |
| detach_workspace        | reverse, with the running-jobs check |
| list_workspaces         | enumerate attached workspaces |
| validate_workspace_path | dry-run path validation for the picker (see §UX) |
```

All four methods sit behind the same bearer gate as every other RPC
(R5). `validate_workspace_path` is server-side rate-limited (token
bucket, ~5/s per connection) so a runaway client can't `stat()`-storm
the disk while a debounced picker is firing.

`ServerInfo.fs_root` (singular) is **frozen to the boot-time
`--fs-root` value** for backwards compat — it does not shift as
workspaces attach/detach. The field is `None` if no flag was passed,
regardless of how many workspaces are attached at runtime. New UI
code reads `list_workspaces`; old clients keep rendering the legacy
banner against the original boot path.

### `validate_workspace_path` — why a separate method

The picker needs to tell the user *before* they click Attach
whether a path is usable. The validator returns a structured result
so the UI can show inline reasons:

```rust
#[derive(Serialize, Deserialize, specta::Type)]
pub struct ValidateWorkspacePathArgs {
    pub path: String,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub struct ValidateWorkspacePathResult {
    pub canonical: Option<String>,
    pub is_dir: bool,
    pub is_git_repo: bool,
    pub default_branch: Option<String>,
    pub already_attached: bool,
    pub readable: bool,
    pub writable: bool,
    pub problems: Vec<WorkspaceProblem>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub enum WorkspaceProblem {
    NotADirectory,
    NotReadable,
    NotWritable,
    NotAGitRepo,
    InsideAnotherWorkspace { other_root: String },
    SystemPath,           // /, /etc, /usr, ~/.ssh, $HOME without subdir, etc.
    SymlinkOutsideHome,
}
```

`SystemPath` is a hard refusal — the server will not attach `/` or
`~/.ssh` even if the user clicks past warnings. `InsideAnotherWorkspace`
is a soft warning unless the user explicitly opts in (see §Edge cases).

## Persistence

New table:

```sql
CREATE TABLE attached_workspaces (
    repo_id           TEXT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    fs_root_canonical TEXT NOT NULL,  -- canonicalised (symlinks resolved, no trailing slash)
    fs_root_display   TEXT NOT NULL,  -- as the user typed it, for UI rendering only
    attached_at       INTEGER NOT NULL -- UnixMillis
);
CREATE UNIQUE INDEX idx_attached_workspaces_canonical
    ON attached_workspaces(fs_root_canonical);
```

The unique index is on the canonical column so symlinks, bind mounts,
and `/var` ↔ `/private/var` aliases collapse to one row. All
attach/upsert paths canonicalise *before* the index check; trailing
slashes and `.` segments cannot create duplicates.

Server boot order:

1. Open the SQLite pool.
2. Read `attached_workspaces`; for each row, register the path with
   the host adapter's allowed-roots list.
3. If `--fs-root` was passed and not already represented, **upsert**
   the row (so the demo flow stays one-command).
4. Start the HTTP listener.

Detach removes the row; subsequent `fs_*` calls under that path
return `PermissionDenied` (not `Internal`, since the path is now a
known-rejected root, not an unconfigured server).

The host adapter also runs a low-frequency liveness sweep (every
~30s) that `stat()`s each canonical `fs_root` and emits
`workspace_unhealthy` if the directory is gone or unreadable, and
`workspace_recovered` when it comes back. Detection is *not* purely
lazy — a workspace the user hasn't touched still surfaces a warning
badge.

## UX — picking a path

The picker is the only thing that legitimately differs by shell. The
UI defines a behaviour interface; each shell injects an
implementation. **No `Foo.web.tsx` / `Foo.desktop.tsx`** (R3) — the
component is one file; the *implementation of the picker function*
is shell-injected.

```ts
// codeless/ui/codeless-ui/src/lib/shell/path-picker.ts
export interface PathPicker {
  /**
   * Show a directory picker and return either an OS-native absolute
   * path, or a user-typed string the *caller* must hand to
   * `validate_workspace_path` before trusting. Returns null if the
   * user cancelled. The contract is deliberately weak so the
   * browser-shell injector can fall back to a typed input where
   * `showDirectoryPicker()` is unavailable — the UI component is
   * identical in both cases.
   */
  pickDirectory(opts?: { startPath?: string }): Promise<string | null>;
}
```

Implementations:

- **Browser shell** — uses `window.showDirectoryPicker()` where
  available (Chromium-family). Firefox/Safari fall back to a typed
  input with `validate_workspace_path` providing live feedback. The
  picker returns the *path* the user typed; the server canonicalises
  and validates it. The browser cannot enumerate the user's disk —
  the user is supplying a path they already know.
- **Tauri desktop shell** — uses `@tauri-apps/plugin-dialog`'s
  `open({ directory: true })`. Returns an absolute path directly;
  no fallback needed.
- **Tauri mobile shell** (Phase 6) — picker is hidden; the workspace
  list is read-only. Mobile users attach via clone-URL (separate flow,
  out of scope here).

If a future shell needs different behaviour, it extends the
interface; the UI never branches on shell identity.

## UX — the Workspaces surface

Route: `/workspaces`. Also surfaced as a **sidebar group** in the
main app shell so the user always sees attached + active workspaces
without leaving their current view.

### Layout

```
┌────────────────────────────────────────────────────────────┐
│ Workspaces                                  [+ Attach]     │
├────────────────────────────────────────────────────────────┤
│ ● codeless    /home/.../codeless           detach   open   │
│   hackline    /home/.../hackline           detach   open   │
│   demo        /tmp/demo                    attach   open   │  ← detached
└────────────────────────────────────────────────────────────┘
```

- **`●`** marks the *active* workspace for this UI tab/window.
- **`open`** switches active to this workspace (no server side-effect).
- **`detach` / `attach`** flips the runtime state via RPC.
- **`+ Attach`** opens the picker → validator → confirm modal.

### Attach modal

```
┌─ Attach a workspace ────────────────────────────────────┐
│ Path:   /home/me/code/myproject              [browse…] │
│         ✓ git repo  ✓ readable  ✓ writable             │
│         default branch: main                            │
│                                                         │
│ Name:   myproject                                       │
│ Runner: claude  ▾                                       │
│                                                         │
│              [Cancel]            [Attach workspace]     │
└─────────────────────────────────────────────────────────┘
```

- **Path** field calls `validate_workspace_path` on every change
  (debounced). Inline checks render as ✓ or ✗ with the problem
  text.
- **Name** auto-fills from the directory basename; user can edit.
- **Runner** dropdown is filtered by `ServerInfo.available_cli_runners`.
- The **Attach workspace** button is disabled until the validator
  returns no `WorkspaceProblem`.
- On click: server runs `add_repo` (if no row yet) + `attach_workspace`
  in one transaction. UI subscribes to `workspaces.*` events and
  updates the sidebar live (R4).

### Detach modal

```
┌─ Detach `codeless` ─────────────────────────────────────┐
│ The following jobs are running against this workspace: │
│   • assistant       (running, 12 min, $1.43 spent)     │
│                                                         │
│ ( ) Leave running — runner keeps writing in worktree,  │
│     but the job page can't show file diffs until you    │
│     re-attach.                                          │
│ (•) Stop them                                           │
│                                                         │
│              [Cancel]                  [Detach]         │
└─────────────────────────────────────────────────────────┘
```

When the workspace has no running jobs, the modal is a one-line
confirm. The two-radio variant only renders when there are running
jobs and the user must make an explicit choice — never silent.

"Leave running" maps to `DetachPolicy::LeaveRunning`. The runner
keeps a private `fs.*` handle scoped to its worktree, but the
**editor** loses access — the job page's live diff / file tree views
go to a "workspace detached, re-attach to view files" placeholder
until the user re-attaches. Stage chat events still stream over
`RpcClient.subscribe()` because they don't traverse `fs.*`.

### Empty state

When `list_workspaces` returns `[]`, the main app shell shows a
blank-state screen:

```
   No workspaces attached.

   Attach a directory on this machine to start working with codeless.

                  [+ Attach a workspace]
```

This replaces the current "fs_root not set" silent failure mode.

## Cross-cutting rules (must hold)

- **R1**: nothing in this surface spawns processes from the UI.
  `validate_workspace_path` runs `git rev-parse` etc. inside the
  host adapter, not in the UI.
- **R2**: only `RpcClient`. The UI does not import
  `@tauri-apps/api/dialog` directly — it goes through the
  `PathPicker` interface defined above.
- **R3**: one responsive component. The picker interface is the
  only shell-visible split, and it's a tiny function injection,
  not a parallel UI tree.
- **R4**: `attached_workspaces` lives in SQLite. The UI subscribes
  to `workspace_attached` / `workspace_detached` events for live
  updates; it does not cache authoritative state.
- **R5**: bearer token authorises attach/detach identically to every
  other RPC. No per-workspace permissions.

## Migration / backwards compat

- `--fs-root <path>` at boot becomes "canonicalise `<path>`, then
  upsert into `attached_workspaces` if no row with that canonical
  path exists". The flag stays so the demo + per-tick scripts keep
  working. Document it as a bootstrap convenience. The boot
  canonicalisation step makes repeated invocations with `/a/b`,
  `/a/b/`, `/a/./b` all collapse to one row.
- Existing `fs.*` RPCs continue to work; they now check the
  attached-roots list (keyed on canonical path) instead of a single
  `Option<PathBuf>`.
- `ServerInfo.fs_root` is frozen to the boot-time `--fs-root` (see
  §RPC additions). It does not shift as workspaces attach/detach.
- One DB migration adds the `attached_workspaces` table and the
  idempotent boot upsert.

## Edge cases — explicit decisions

- **Nested repos** (a workspace path inside another). Refuse by
  default with `InsideAnotherWorkspace`; allow with an explicit
  "yes, attach the sub-tree separately" override in the picker.
  Reason: the `fs.*` canonicalisation needs unambiguous root
  resolution.
- **Attach a non-git directory.** Allowed (validator returns
  `is_git_repo: false` as a *warning*, not a problem). The job
  runner refuses to use a non-git workspace because worktrees need
  git; that's surfaced when the user submits a job, not at attach
  time. Editor-only attach is a legitimate use case.
- **Path moves on disk after attach.** The server detects the missing
  directory on the next `fs.*` call and emits a
  `workspace_unhealthy` event with the canonical path. The UI shows
  the workspace with a warning badge; the user can detach or fix it.
- **Two clients attach the same path simultaneously.** The unique
  index on `attached_workspaces.fs_root` makes the second call a
  no-op (`Conflict` returned, the row already exists). UI treats
  `Conflict` as "already attached" and renders accordingly.
- **Browser tab open against a detached workspace.** UI receives the
  `workspace_detached` event and switches active to the
  most-recently-attached remaining workspace (by `attached_at`
  descending), or to the empty state if none remain. No silent
  failure.
- **No runners installed** (`ServerInfo.available_cli_runners` is
  empty). Attach still proceeds — editor-only attach is a valid use
  case. The Runner dropdown renders disabled with a "no runners
  installed" hint and `default_runner` is stored as `None`.

## Data the UI needs (events)

```
| Event                    | Payload                 | UI reaction |
|--------------------------|-------------------------|-------------|
| workspace_attached       | AttachedWorkspace       | append to sidebar; toast |
| workspace_detached       | { repo_id }             | remove from sidebar; if active, switch |
| workspace_unhealthy      | { repo_id, reason }     | warning badge on the row |
| workspace_recovered      | { repo_id }             | clear the badge |
```

All ride the existing `RpcClient.subscribe()` channel (R4). No new
transport.

## Open questions

> **Status (2026-05-15):** all four resolved in line with the
> recorded biases during the workspace-attach stage 1. Per-question
> reasoning lives in
> [`codeless/.codeless/jobs/workspace-attach/SCOPE.md`](../codeless/.codeless/jobs/workspace-attach/SCOPE.md)
> §"Open questions"; the one-liners below capture *what* was
> chosen for readers of this doc. Revisit triggers are noted inline.

1. Should the `--fs-root` flag be **removed** in favour of the demo
   flow being "start the server, then attach"? Bias: keep it for now;
   too many docs / scripts depend on it. Revisit when the wrapper
   ([`codeless/setup/init-session.sh`](../codeless/setup/init-session.sh))
   absorbs the auto-attach via API.
   - **Resolved: keep.** Boot canonicalises the value and upserts a
     row into `attached_workspaces`; the flag stays a bootstrap
     convenience until `init-session.sh` can auto-attach via the new
     RPC.
2. **Where does `worktree-root` live now?** Today it's a server-wide
   flag. Per-workspace would let the user keep all worktrees for
   `codeless/` under `~/dev/.worktrees/codeless/` and all for
   `hackline/` under `~/dev/.worktrees/hackline/`. Bias: deferred —
   the current single root is fine, revisit if users complain.
   Coupling note: if worktree-root becomes per-workspace, `detach`
   must also decide whether to GC the workspace's worktrees (and
   `DetachPolicy::LeaveRunning` interacts with that — running jobs
   still hold their worktree open). Resolve together.
   - **Resolved: defer.** Stays server-wide. The schema change ships
     together with the detach-time GC policy that
     `DetachPolicy::LeaveRunning` forces; piecemeal is rejected.
3. Should detach **archive** the repo row or leave it as
   "registered, detached"? Bias: leave it. `remove_repo` is the
   destructive verb; detach is reversible.
   - **Resolved: leave.** Detach removes only the
     `attached_workspaces` row. The `repos` row is the named handle
     re-attach binds to; destruction stays in `remove_repo`.
4. Does the desktop shell need a **drag-and-drop** affordance ("drop
   a folder onto the window to attach")? Bias: not in milestone 1.
   Add later if the UX testing surfaces it.
   - **Resolved: no.** Picker + `validate_workspace_path` covers
     attach on every shell. Revisit after milestone 4 picker UX
     testing.

All four resolved before milestone 2 began.

## Milestones

Status legend: `[x]` done, `[~]` partial, `[ ]` not started.

1. `[x]` **Decisions.** Resolve the four open questions; record in this
   file. No code.
   _Landed: workspace-attach job stage 1 (`0e4eeb1`)._
2. `[x]` **Server-side.** Add `attached_workspaces` table, the four RPC
   methods, the boot-time auto-attach, and the host-adapter switch
   from `Option<PathBuf>` to allowed-roots list. `cargo test` round-trips
   attach → list → detach. UI unchanged.
   _Landed: workspace-attach job stages 3-7, merged via PR #6
   (`fae135c`, `54027ac`, `a77056a`, `4016cdf`, `c36d4d2`)._
3. `[~]` **Pickup in `RpcClient`.** Generate the new wire types; add the
   four methods to `RpcClient`. Browser + Tauri shells inject their
   `PathPicker` implementations. No UI yet.
   _Done: TS wire types generated into
   `ui/codeless-ui/src/lib/rpc/generated/wire.ts` via specta.
   Missing: `attachWorkspace` / `detachWorkspace` / `listWorkspaces`
   / `validateWorkspacePath` on the `RpcClient` interface and both
   shell implementations (`HttpSseClient`, `TauriIpcClient`); the
   `PathPicker` shell capability and its two injectors; the typed-wire
   snapshot test for the four methods._
4. `[ ]` **Workspaces page.** Build `/workspaces`, the sidebar group, and
   the empty-state screen. Hook attach/detach modals through the
   picker + validator.
   _Phasing decision (2026-05-15): a Settings → Workspaces tab ships
   first as a smaller landing surface, then the `/workspaces` route
   and sidebar group follow. Both share the same components; the
   tab is not a parallel UI._
5. `[ ]` **Job-page integration.** Filter the jobs view by active
   workspace; show a "switch workspace" affordance when the user
   clicks a job from a different workspace. Requires the per-tab
   active-workspace store from M4.
6. `[ ]` **Health & events.** Wire `workspace_unhealthy` /
   `workspace_recovered` from the host adapter; render badges +
   recovery flow. Server-side emits exist (stage 7); UI does not
   subscribe yet.

Each milestone ships behind the same UI route; partial completion is
visible but not feature-flagged.

**Test exit criteria.** Milestone 2: Rust round-trip
`attach → list → detach`, plus a canonicalisation test
(`/a/b`, `/a/b/`, symlink to `/a/b` all collapse to one row).
Milestone 3: a typed-wire snapshot test for the four methods.
Milestones 4–6: at minimum one Playwright/RTL happy-path test per
milestone (attach modal, switch active, unhealthy badge). No
milestone lands without its exit test.
