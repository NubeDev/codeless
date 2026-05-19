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

## Port binding — running multiple instances

Attaching N workspaces inside one server is the primary multi-tenant
story. But the user also runs the *server itself* more than once:
one per dogfood run, one per integration test, one per
`codeless-tauri-desktop` window backed by an embedded `codeless serve`
sidecar, plus whatever ad-hoc instance the developer has open in a
terminal. Hardcoding `127.0.0.1:7777` makes the second `codeless
serve` crash with `AddrInUse` for no reason the user can act on.

The fix is the same trick Vite, esbuild, and every modern dev tool
use: bind to port `0` and let the OS pick. The port is reported back
to the caller (stdout banner, optional `--port-file`, and the existing
`on_bound` callback in `codeless-server`).

### Surface

| Surface             | Today                      | Target                                                |
| ------------------- | -------------------------- | ----------------------------------------------------- |
| `codeless serve`    | `--bind 127.0.0.1:7777`    | default loopback + ephemeral; `--bind` is an override |
| `codeless-tauri-desktop` | in-process IPC (no port) | unchanged — never opens a TCP port                    |
| `codeless-mcp`      | stdio (no port)            | unchanged — stdio handshake, one process per client   |
| Test harnesses      | mixed (some hardcode)      | all route through the shared helper                   |

`codeless-tauri-desktop` invokes `boot::boot()` over Tauri IPC; there
is no TCP transport to assign a port to. `codeless-mcp`'s `rmcp`
transport is `transport-io` (stdin/stdout) — each MCP client spawns
its own child, so "multi-instance" is the default and no port is
involved. Both are explicitly out of scope for this change; they show
up in the table so the next reader does not waste an afternoon
looking for the port that does not exist. If a future MCP HTTP/SSE
transport lands, it reuses the same helper as `codeless serve`.

### Reusable helper

Lives in `codeless-adapters-host::net` (host-only, per
[SCOPE.md](./SCOPE.md) crate layout) so any host binary can pull it
in without the mobile-safe crates picking up a `tokio::net`
dependency:

```rust
pub async fn bind_tcp(
    requested: Option<SocketAddr>,
) -> std::io::Result<(tokio::net::TcpListener, SocketAddr)>;
```

- `requested = None` → binds `127.0.0.1:0`; OS assigns a free port
  atomically. This is the *only* race-free way to claim a port;
  "find a free port, then bind to it" has a TOCTOU window the
  helper exists to forbid.
- `requested = Some(addr)` → pins the address; returns `AddrInUse`
  on conflict. Callers decide whether to fall back to `None` or
  surface the error (CLI surfaces it; sidecar boot falls back).
- Returns the *live* listener. Callers must not drop and re-bind to
  the reported address — that reintroduces the TOCTOU race.

`codeless-server::serve_with_shutdown` already accepts a SocketAddr
and reports the bound addr via its `on_bound` callback; the change
there is mechanical (take a pre-bound `TcpListener` instead, or call
`bind_tcp` internally) and lands with the CLI change below.

### CLI surface

```
codeless serve [--bind <ADDR>] [--port-file <PATH>]
```

- `--bind` default flips from `127.0.0.1:7777` to *unset*; unset means
  `bind_tcp(None)`. Passing `--bind 127.0.0.1:7777` restores today's
  behaviour for scripts that need a stable port.
- `--port-file <PATH>` (new): after bind, atomically write the chosen
  `host:port` to `<PATH>` (tmp-file + rename). Discovery for tests,
  Tauri sidecar supervisor, and `codeless-cli` client commands that
  want to talk to "the local server I just started" without parsing
  stdout.
- Stdout banner stays: `codeless-server listening on http://{addr}`
  is the human-readable equivalent of `--port-file`.

### Auth gate is unchanged

`codeless-cli/src/serve.rs` lines 183–205 refuse non-loopback binds
without `--require-token`. Ephemeral ports on `127.0.0.1` satisfy the
loopback check, so the gate keeps working untouched. Pinning a
non-loopback address still requires `--require-token`. The helper
does not encode auth policy; that stays in the CLI.

### Out of scope

- Service discovery beyond `--port-file` + stdout banner. mDNS, a
  registry file in `~/.codeless`, or a "list running servers" CLI
  are follow-ups, not this change.
- Port *reservation* across restarts. Each `codeless serve` invocation
  re-binds; whatever the OS picks is what you get. A future "sticky
  port per workspace" feature can persist the last-bound port into
  `attached_workspaces` and pass it to `bind_tcp` as `Some(addr)`
  with fallback — but that needs the multi-instance UX to settle
  first.

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
7. `[~]` **Port binding.** Land `codeless-adapters-host::net::bind_tcp`
   (done; 3 unit tests). Flip `codeless serve --bind` default to
   unset (→ ephemeral loopback), thread the pre-bound listener into
   `serve_with_shutdown`, and add `--port-file <PATH>` discovery.
   Update sidecar/test callers to use the helper instead of
   hardcoded ports. Exit test: two `codeless serve` instances start
   concurrently without `AddrInUse`; `--port-file` content matches
   the stdout banner.

Each milestone ships behind the same UI route; partial completion is
visible but not feature-flagged.

**Test exit criteria.** Milestone 2: Rust round-trip
`attach → list → detach`, plus a canonicalisation test
(`/a/b`, `/a/b/`, symlink to `/a/b` all collapse to one row).
Milestone 3: a typed-wire snapshot test for the four methods.
Milestones 4–6: at minimum one Playwright/RTL happy-path test per
milestone (attach modal, switch active, unhealthy badge). No
milestone lands without its exit test.

## TODO — adapter registry (chat adapters + AI runners)

Follow-on surface, not in scope for the workspace-attach milestones
above. Captured here so it does not get lost; will graduate to its
own `SCOPE-ADAPTER-REGISTRY.md` when picked up.

**Goal.** Let the user enable/disable and configure the **chat
adapters** (Slack, Telegram, Gmail) and **AI runners** (`claude`,
`anthropic`, `codex`, `copilot`) from the UI, the same way
workspace-attach replaced `--fs-root`. Today both are boot-time CLI
flags (`--enable-slack`, `--enable-telegram`, `--enable-claude`, …)
read once by [`codeless-cli/src/serve.rs`](../crates/codeless-cli/src/serve.rs)
and baked into [`DefaultRunnerFactory`](../crates/codeless-runtime/src/default_runner_factory.rs)
+ adapter spawn calls. Changing any of them needs a manual restart
with edited flags.

**Stage 1 — reboot is acceptable.** Server restart on apply is
explicitly fine; SQLite is the source of truth (R4) and the job
driver already replays the backlog on boot
([`job_driver_loop::replay_backlog`](../crates/codeless-runtime/src/job_driver_loop.rs)).
That removes the `Arc<ArcSwap<…>>` / per-adapter graceful-shutdown
work and shrinks the milestone to:

Server-side milestone (1-5) landed in `codeless/adapter-registry`;
follow-up jobs own (3) keyring backend rollout, (6) Settings UI, and
the Gmail adapter. Stage 2 hot-reload remains deferred until a trigger
fires.

1. `[x]` SQLite tables: `chat_adapters(kind, instance_id, enabled,
   configured_at, PRIMARY KEY (kind, instance_id))` and
   `runner_config(runner_id PRIMARY KEY, enabled)`. Boot reads these
   instead of CLI flags. `--enable-*` flags stay as bootstrap
   conveniences that upsert the row (same pattern as `--fs-root` →
   `attached_workspaces`).
   - **Why two tables, not one** (peer-review pushback): chat
     adapters and runners have different config shapes. Runners are
     ~enable + optional binary path. Chat adapters carry per-
     instance config (workspace IDs, channel filters, mailbox).
     One `enabled_components(kind, id, enabled, config_json)` table
     is closer to how workspaces did it; if the per-adapter config
     stays small enough to live in a `config_json` blob, collapse
     to one table when graduating to `SCOPE-ADAPTER-REGISTRY.md`.
   - **Composite PK on `(kind, instance_id)`** so the user can have
     Slack-personal + Slack-work, or two Gmail accounts, without a
     schema change. Default `instance_id = "default"` covers the
     today-case.
2. `[x]` Secrets go through the existing `SecretStore`
   ([`codeless-adapters-host/src/secrets.rs`](../crates/codeless-adapters-host/src/secrets.rs))
   — XDG-pathed TOML at `~/.config/codeless/secrets.toml`, flat
   `key = "value"` map, already used for `slack_*`,
   `telegram_bot_token`, `anthropic_api_key`. New keys:
   `gmail_refresh_token`, `gmail_client_id`, `gmail_client_secret`.
   The new tables hold the enable bit only; secrets writes go
   through the existing secrets RPC. Write-then-fsync-then-restart
   ordering is load-bearing — UI must not trigger restart before
   the secrets file is durable on disk.
3. `[x]` Harden the secrets store. Today's file is plaintext TOML
   with no at-rest protection beyond filesystem perms. The Gmail
   refresh token (long-lived, account-wide mail access) raises the
   blast radius enough that this should ship alongside the Gmail
   adapter, not after. Options, in order of preference:
   - OS keychain via the `keyring` crate (Secret Service on Linux,
     Keychain on macOS, Credential Manager on Windows). Per-key
     entries; `SecretStore` becomes a thin facade. Best UX, no
     passphrase prompt, ties unlock to the desktop session.
     Headless Linux servers fall back to (b).
     `codeless-tauri-desktop` already brokers OS-integration
     calls; this fits the same shape.
   - `age`-encrypted file under a passphrase the user enters once
     per server boot. Works headless; adds a startup prompt and a
     "secrets locked" failure mode that the UI has to handle.
     Worth doing as the fallback path for the keychain story.

   Sequence: land the keychain backend behind a `SecretBackend`
   trait so `SecretStore`'s callers don't care which is in use,
   then migrate existing keys on first read. The TOML file stays
   as a `--secrets-file` opt-in for CI / fixtures.
4. `[x]` RPC: `list_chat_adapters`, `set_chat_adapter_enabled`,
   `validate_chat_adapter_secrets` (calls `auth.test` for Slack,
   `getMe` for Telegram, token introspection for Gmail);
   `list_runners`, `set_runner_enabled`. All behind the bearer gate
   (R5).
   - **Validate-before-enable coupling.**
     `set_chat_adapter_enabled(true)` MUST refuse with
     `MissingSecrets { keys: Vec<String> }` or
     `ValidationFailed { reason }` unless a prior
     `validate_chat_adapter_secrets` for the same `(kind,
     instance_id)` succeeded within the current session. Otherwise
     a restart succeeds, the adapter crashes on startup, and the
     UI has no clean error path.
   - **Timeouts and rate limiting.**
     `validate_chat_adapter_secrets` calls (Telegram `getMe` over
     a fresh bot token can hang ~30s on a bad network) have a
     hard 5s timeout. Rate limit is per-`(kind, instance_id)`
     bucket (5/s), not per-connection — otherwise a slow Slack
     validation blocks a concurrent Telegram one. This is a
     deliberate divergence from `validate_workspace_path`'s
     per-connection limit; the precedent does not fit here.
5. `[x]` `restart_server` RPC. Three contexts, not two; the
   peer-review caught that the browser-shell case was unaddressed.
   - **CLI under a supervisor** (systemd, `init-session.sh`,
     `--respawn-on-exit` — see below): exit with sentinel code 75
     `EX_TEMPFAIL`, supervisor re-execs. Or
     `std::os::unix::process::CommandExt::exec` for in-place
     replacement when the supervisor is `init-session.sh`-shaped
     rather than systemd-shaped.
   - **Tauri desktop**: shell owns the `codeless serve` sidecar,
     kills and respawns, shows a "restarting" toast while the SSE
     channel reconnects.
   - **Bare `codeless serve` in a terminal** (browser-shell user
     with no supervisor): the RPC returns
     `RestartUnsupervised { hint }` and refuses to exit. Two ways
     to make it work:
     a. Add `codeless serve --respawn-on-exit`, which spawns a
        thin self-watcher (parent process re-execs the child on
        exit-75). This is the recommended path — it makes the
        standalone case Just Work without external tooling.
     b. Document the hint as "restart manually" with a copy-pasta
        command. Acceptable fallback; not the default story.
   The current draft implied this Just Works in all contexts; it
   does not. Pick (a) before the milestone closes.
   - **In-flight job impact.** Restart is not free for running
     jobs. `claude` / `codex` / `copilot` runners are PTY-bound;
     the child process dies on restart, the stage's last
     checkpoint replays, but anything between the checkpoint and
     the restart is lost. The `restart_server` precondition:
     enumerate running jobs and partition into *resumable*
     (template-driven, checkpoint within last N seconds) vs
     *killed* (mid-PTY-stream, no recent checkpoint). The verb
     returns `RestartHasRunningJobs { resumable: Vec<JobId>,
     killed: Vec<JobId> }` unless called with `force: true`. The
     confirm modal at step 6 renders this partition before the
     user clicks Apply. No `DetachPolicy::LeaveRunning` analogue —
     the server is going down, there is no "leave running"
     option — but the user must see which jobs will be killed,
     not just told "jobs will pause and resume".
   UI batches "Apply" so three toggle changes = one restart, not
   three.
6. `[ ]` Settings → Adapters page. One responsive component (R3):
   rows for each chat adapter and runner, toggle + secret fields,
   "Apply (will restart)" button with confirm modal that names the
   running jobs that will pause/resume.

**Stage 2 — hot-reload (deferred, optional).** Lift adapter
lifecycle out of `serve.rs` into a `ChatAdapterRegistry` owned by
`ServerState`; wrap the `DefaultRunnerFactory` `enable_*` fields in
`AtomicBool` or behind an `ArcSwap<RunnerConfig>`. Removes the
restart on apply. Triggers (any one promotes stage 2 from
"deferred" to "next milestone"):

- More than ~5 `restart_server`-initiated job kills per week in
  dogfood telemetry (i.e. the in-flight-impact warning above is
  actually firing).
- iOS / Android shell lifecycle requirements that make restart
  user-visible (mobile suspend / resume crosses a server restart
  boundary).
- A user-facing "rotate Slack token without dropping the active
  job" ask that lands in the issue tracker more than once.

Until one of those triggers fires, stage 1 is the whole story.
Without a measurable trigger this stays deferred forever, which
is the failure mode the peer review flagged.

Stage 2 is a **separate follow-up job** when a trigger fires; the
`ChatAdapterRegistry` and `RunnerConfig` shapes that stage 1 landed
in `codeless/adapter-registry` are the seams it will swap behind an
`Arc<ArcSwap<…>>`. Until then, the SQLite-source-of-truth +
`restart_server` partition is the supported path.

**Pluggability** (also peer-review). The closed set today is
Slack, Telegram, Gmail — and the registry's `ChatAdapterRegistry`
trait is the explicit extension point. Adding Discord / Matrix /
SMS later means a new `codeless-discord` crate that implements
`BotTransport` (from `codeless-bot-core`) and registers itself
with the registry at boot; no schema change because of the
`(kind, instance_id)` PK from step 1, no RPC change because
`list_chat_adapters` enumerates whatever is registered. Recompile
required — adapters are not WASM plugins, and that is a
deliberate commitment for the foreseeable future.

**Gmail-specific work (separate follow-up job, slots into the
registry).** The registry surface stage 1 shipped is what the Gmail
crate plugs into; no `Gmail` variant exists in `ChatAdapterKind` yet
— it lands paired with the new crate. Outbound is already done:
[`codeless-tools::email::GmailMailer`](../crates/codeless-tools/src/email/gmail.rs)
posts to `users.messages.send` with a caller-supplied OAuth2 token,
and the `Mailer` trait is transport-agnostic. What's missing for
Gmail-as-chat-adapter:

1. `[ ]` OAuth host wiring — PKCE flow + refresh token, stored via
   the hardened `SecretBackend` from stage 1 step 3. Token
   acquisition is deliberately out of scope for `codeless-tools`;
   this lands in a new `codeless-gmail` crate paralleling
   `codeless-slack` / `codeless-telegram`. (`BotTransport` lives in
   [`codeless-bot-core`](../crates/codeless-bot-core/) — the new
   crate implements that trait.)
   - **Refresh-token rotation.** Google rotates the refresh token
     on some flows (scope changes, long inactivity, security
     events). The `SecretBackend` needs update-in-place semantics
     (already present) *and* a `secrets_changed` event the
     inbound long-poll subscribes to so it picks up the new token
     without a restart. Without this the long-poll silently uses
     a stale token after rotation and inbound mail goes dark.
2. `[ ]` Inbound transport — long-poll `users.history.list` against
   a stored `historyId`. Mirrors the existing Telegram long-poll
   pattern ([`codeless-telegram/src/long_poll.rs`](../crates/codeless-telegram/src/long_poll.rs))
   and avoids the public-webhook dependency that `users.watch` +
   Pub/Sub would impose on a local-first tool.
3. `[ ]` `BotTransport` impl + envelope mapping via
   `codeless-bot-core`. Reuse the parser; the new piece is
   `Message-ID` / `In-Reply-To` / `References` → `ThreadMap`
   (replacing Slack's `thread_ts` / Telegram's
   `message_thread_id`).
4. `[ ]` Wire outbound through the existing `GmailMailer` — `Message`
   construction in the adapter, hand to the already-built sender.

Stage 1 of the registry plus the Gmail crate together replace every
`--enable-*` flag with a Settings page row.

**Exit tests (peer-review).** No milestone closes without:

- Write-then-fsync-then-restart ordering: a unit test that crashes
  the process between secrets-write and restart-signal proves the
  on-disk state is durable. Today's "load-bearing comment" is not
  enough.
- `restart_server` partition test: a job with a recent checkpoint
  is reported `resumable` and resumes; a job mid-PTY-stream is
  reported `killed` and the kill is logged with a structured event
  the UI can render post-restart.
- `set_chat_adapter_enabled(true)` without a prior successful
  `validate_chat_adapter_secrets` returns the structured
  `MissingSecrets` / `ValidationFailed` error, not a generic
  `Conflict`.

## TODO — multi-window desktop isolation

Status: open. Owner: ap@nube-io.com. Raised: 2026-05-19.

### Symptom

Two `codeless-tauri-desktop` windows opened by the user appear to
share state — the attached-workspaces list, the job list, and the
event stream all look identical between windows, even after the user
"attaches" what should be a different workspace in one of them. The
user expects a VSCode-style model: open many windows, each one
scoped to its own workspace, jobs and events visible only in the
window that owns them.

### Root cause

`codeless-tauri-desktop` boots a fresh `InProcessRpc` (SQLite +
event bus + driver loop + REST sidecar) per process. Two launches of
the binary mean two processes, both opening the same on-disk SQLite
file (one global path under `~/.local/share/codeless/` historically,
or — after the 2026-05-19 per-workspace-slug patch in
[`crates/codeless-tauri-desktop/src/boot.rs`](../crates/codeless-tauri-desktop/src/boot.rs)
— the same `~/.codeless/workspaces/<slug>/codeless.sqlite` whenever
both launches resolve to the same workspace slug). Two processes
opening one SQLite file means:

- Two driver loops racing one `jobs` queue.
- Two stage recorders subscribing to one bus.
- Two liveness sweeps stat'ing the same allowed-roots set.
- One `attached_workspaces` table that both windows read, so
  "attach workspace X in window B" is immediately visible in
  window A — the windows are not isolated, they are sharing
  state through SQLite.

The per-workspace-slug patch fixes the *cross-workspace* case (two
binaries launched against `~/code/foo` and `~/code/bar` get
different DBs and no longer collide) but does not fix the
*same-workspace-twice* case nor the case the user actually has,
which is "I want two windows side by side, each one on a different
workspace from my library, no cross-talk." That case fails because:

1. The user's launch flow is "double-click the binary from the
   `dist/` folder," which produces `cwd = .../dist/` on both
   launches → same slug → same SQLite.
2. The UI's "attach workspace" verb writes a row into the running
   process's `attached_workspaces` table — it does not change which
   workspace this *process* is scoped to. The UI's `activeRepoId`
   state (per-tab in zustand) selects which attached workspace the
   window is currently *viewing*, but jobs / events / queue are not
   filtered by `activeRepoId` on the server side, so the second
   window still sees everything the first one is doing.

The mismatch is between the model the per-workspace-slug patch
assumed ("one process = one workspace, `--workspace` at launch
decides which") and the model the UI was built for ("one process,
one DB, many attached workspaces, `activeRepoId` selects the view").
The UI's model is the correct one for the product; the patch solved
the wrong problem.

### Proposal — direction of work

Adopt the VSCode model:

1. **Single-instance lock** on the desktop binary. A second launch
   does not spawn a second runtime; it forwards its launch
   arguments to the first instance via
   [`tauri-plugin-single-instance`](https://v2.tauri.app/plugin/single-instance/),
   which then opens a new Tauri window inside the running process.
   One DB, one driver, one event bus — many windows.
2. **Revert the per-workspace data-dir patch.** The
   `~/.codeless/workspaces/<slug>/` layout from the 2026-05-19
   change is not load-bearing under the single-instance model; one
   shared `~/.codeless/codeless.sqlite` is enough. (The
   `--workspace` CLI arg can stay as a way to tell a freshly-
   focused window which workspace to open by default, but it stops
   selecting a different DB.)
3. **Server-side scoping by `repo_id`.** The UI's per-window
   `activeRepoId` becomes an *RPC parameter*, not just a UI
   filter. Job-list, stage-event subscriptions, and the worktree-
   write surface all take `repo_id` and the runtime filters
   server-side so window A literally cannot observe window B's
   activity. This is a non-trivial RPC change — most read methods
   today are workspace-agnostic — and is the bulk of the work.
   Without it, single-instance just means "windows still share the
   same firehose," which the user already finds confusing.
4. **`attached_workspaces` keeps its current meaning** — it's the
   user's library of known workspaces. Attach adds a row. Detach
   removes one. Windows pick from this list via `activeRepoId`.
5. **`worktrees/` stays global** under `~/.codeless/worktrees/`,
   keyed by job ID (already the case). Multiple workspaces sharing
   one worktree root is fine because job IDs are globally unique.

Long-term this also subsumes the per-workspace data-dir idea: if a
user later wants two *fully isolated* codeless installations (one
for work, one for personal), the single-instance lock can be
per-data-dir, controlled by a `CODELESS_HOME` env var defaulting to
`~/.codeless`. Out of scope for this TODO; mentioned so the design
does not foreclose it.

### Migration

- `~/.codeless/workspaces/<slug>/codeless.sqlite` artifacts created
  by the 2026-05-19 patch are orphaned by this rollback. A boot
  shim that finds them and merges `attached_workspaces` rows into
  the global `~/.codeless/codeless.sqlite` would be a kindness but
  is optional — the user can re-attach by hand. Decide based on
  whether dogfood instances have meaningful state in those dirs by
  the time this lands.
- The historical `~/.local/share/codeless/codeless.sqlite` (used
  before 2026-05-19) is also orphaned. Same call: merge or
  document-and-discard.

### Exit criteria

- Two desktop windows open against two different attached
  workspaces. Submitting a job in window A does *not* surface in
  window B's job list or event stream.
- Attaching a workspace in window A *does* surface in window B's
  picker (it's a library-level change, not a window-level one).
- Closing window A does not stop window B's jobs.
- Closing the last window still keeps the runtime alive long
  enough for any in-flight job to checkpoint (existing graceful-
  shutdown semantics).
- `tauri-plugin-single-instance` callback runs end-to-end on
  Linux, macOS, and Windows targets in CI.

### Open questions for peer review

1. Is single-instance + server-side `repo_id` scoping the right
   shape, or is there a simpler win we're missing? (E.g. a thin
   "session" abstraction that filters events client-side without
   server-side changes — quicker to ship, leaks data through
   side channels.)
2. Which existing RPCs need `repo_id` plumbed through? A full
   audit lands in the implementing job; the question now is
   whether the surface area is small enough that this is a
   one-job change or large enough that it needs its own scope
   doc.
3. Is the orphaned-data migration worth writing, or is "user
   re-attaches by hand" the correct trade-off given how new the
   per-workspace-slug patch is?
4. Does the single-instance model break anything Tauri does for
   us today? (Window state persistence, deep-link handling, dock
   icon behaviour — anything that assumes "one process per
   window".)
5. What happens to the embedded REST sidecar (`codeless-server`
   bound on an ephemeral loopback port)? Under single-instance
   there is one REST endpoint per *process*, not per window —
   external tools targeting `ServerInfo.rest_url` will see the
   union of all windows' state unless `repo_id` is a query
   parameter on every REST route, mirroring the RPC change.
