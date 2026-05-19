# BROWSER-LAUNCHER — Scope

Status: proposed, not started
Owner: ap@nube-io.com
Created: 2026-05-19

## Summary

Today `codeless-tauri-desktop` is a Tauri 2 native-window shell with
an embedded `InProcessRpc`. Every launch is its own process, its own
SQLite, its own driver loop, its own event bus — opening two windows
means two runtimes racing over the same files and the user sees
cross-talk between what should be independent workspaces. The
per-workspace data-dir patch (2026-05-19,
[`src/boot.rs`](./src/boot.rs)) papered over one shape of the bug
but does not match the model the UI was actually built for.

> **Platform scope.** This milestone proves the model on **Linux
> only**. macOS and Windows ship after Linux is proven end-to-end
> (two tabs, two workspaces, zero cross-talk, tray quit drains
> jobs cleanly). The cross-platform UX surfaces — "open browser",
> tray, code-signing, dock-reopen — are called out below where
> they differ, but their resolution is deferred to a follow-on
> milestone. Linux is where the dogfood lives; Linux is what
> gates the design.

This doc proposes flipping the desktop shell from
**native-window-per-process** to **launcher-with-browser-tabs**:

- One desktop binary launch boots **one** `InProcessRpc` runtime and
  the REST sidecar that already lives at
  [`src/main.rs`](./src/main.rs).
- Instead of opening a Tauri webview, the launcher opens the user's
  **default browser** at the bound REST URL.
- Every new tab the user opens is a fresh UI client of that one
  runtime. Per-tab `activeRepoId` (already in the UI's zustand
  store) scopes which workspace the tab is viewing.
- The server filters event subscriptions and job lists by `repo_id`
  so two tabs on two workspaces never observe each other's
  activity.
- Tauri's role shrinks to: boot the runtime, open the browser, sit
  in the system tray, exit cleanly on quit.

Net result: a user can run "many independent desktop sessions" by
opening many browser tabs, with zero crossover, no multi-process
SQLite races, and no Tauri-window-lifecycle workstream.

> **Sister docs.** This is a follow-up to
> [`../../DOCS/WORKSPACE-ATTACH.md`](../../DOCS/WORKSPACE-ATTACH.md)
> §"TODO — multi-window desktop isolation", which captured the bug
> and the alternative single-instance proposal that this doc
> supersedes. The UI architecture rules this complies with are
> [`../../DOCS/UI-ARCHITECTURE.md`](../../DOCS/UI-ARCHITECTURE.md).
> Where this doc disagrees with either, **those win** — open an
> issue and update this file.

## Why this and not single-instance

The peer review on the WORKSPACE-ATTACH TODO recommended
single-instance + server-side `repo_id` scoping: one process, many
Tauri windows, route argv through `tauri-plugin-single-instance`,
pin window labels to `workspace-<repo_id>`, handle macOS dock
`Reopen`, audit ~6 RPCs. That works.

This doc proposes the alternative because:

1. **The bug becomes a startup-race window of milliseconds, not a
   steady-state race.** Two browser tabs cannot fight over a
   SQLite file because there is no second runtime under normal
   operation. The remaining race is "two `cargo run` invocations
   before the port-file is written" or "two users on the same
   machine"; that surface is mitigated by an advisory `flock(2)`
   on the port-file (see §Port-file), which is atomic and releases
   on process death — no TOCTOU between `kill -0` and `bind`. Not
   structurally impossible; structurally small and bounded.
2. **It matches the architecture already in the repo.** R2 in
   [`../../codeless/CLAUDE.md`](../../CLAUDE.md) says the UI imports
   only `RpcClient` and the shell injects the impl.
   `HttpSseClient` is already the canonical impl for browser +
   mobile (Phase 6). Going browser-first on desktop means three of
   four shells share one transport — the four-shell story
   simplifies, not complicates.
3. **It does not fight Tauri's "one process per window"
   assumption.** Single-instance + many-windows-one-process needs
   `tauri-plugin-single-instance`, stable window labels, dock
   reopen handlers, per-window state persistence — all work that
   the reviewer flagged. Browser tabs are exactly what browsers
   already do.
4. **Mobile and remote-server stories ride the same rails.**
   When a hosted multi-user version lands later (Phase 7 in
   SCOPE.md), the browser tab on `https://codeless.example.com` is
   the same UI talking to the same transport, just pointed at a
   different URL. The native Tauri webview becomes the special
   case, not the default.
5. **Server-side `repo_id` scoping is required in both models.**
   This isn't extra work — it's the work either way.

## Goals

1. Launch the desktop binary; it boots `InProcessRpc` + REST
   sidecar; the user's default browser opens at the bound URL with
   no further interaction.
2. Open another browser tab against the same URL → a second
   independent UI session; pick a different workspace; submit a
   job; the first tab does not see it.
3. Close every tab; the launcher stays running in the system tray
   so in-flight jobs continue. Click the tray to open a fresh tab.
4. Quit from the tray; in-flight jobs reach their next checkpoint
   and the runtime exits cleanly.
5. Single-binary distribution survives: user double-clicks one
   binary, gets one running app + one browser tab. Same UX as
   Jupyter, Vite dev server, every modern local-first tool.
6. The native-window codepath (`TauriIpcClient`) stays in the repo
   behind a `--native-window` flag for the day someone wants
   integrated-window feel. Not the default, not load-bearing.

## Non-goals

- Removing Tauri. The launcher uses Tauri for the tray icon, the
  REST sidecar lifecycle, and the cross-platform "open the
  browser" affordance. Replacing it with a bare CLI binary is a
  later question.
- Hosted multi-user deployments. This doc covers single-user,
  same-host. Auth-on-loopback stays default-off; the trust
  boundary is R5 (single tenant) just like today.
- A bundled browser. The launcher uses the user's default
  browser. If a user has no default browser, the launcher prints
  the URL and waits.
- iOS / Android shells. Phase 6. Those are already `HttpSseClient`
  + native shell injectors; this doc does not change their
  trajectory.
- Removing `TauriIpcClient`. It stays as the optional
  `--native-window` transport.

## Concepts

### Launcher vs. window

- **Launcher** = the OS process the user starts. Owns the
  `InProcessRpc`, the SQLite file, the driver loop, the event bus,
  the REST sidecar, the system tray icon. There is **one** per
  user session.
- **Window** = a browser tab. Owns nothing on the server. Holds
  per-tab UI state (`activeRepoId`, scroll position, expanded
  panels) in the browser's tab-local memory. The user opens as
  many as they want. Each is an independent client of the
  launcher.

### Workspace selection per tab

`activeRepoId` is already a per-tab zustand store (see the peer
review's reference to
`ui/codeless-ui/src/modules/workspaces/store.ts`). Under this
model:

- Tab A picks `activeRepoId = repo_42`.
- Tab B picks `activeRepoId = repo_99`.
- Both tabs share the launcher's `attached_workspaces` library —
  attaching a new workspace in tab A surfaces in tab B's picker
  (library-level change, R4).
- Job submission, file-tree views, and stage event subscriptions
  in tab A scope to `repo_42`; tab B scopes to `repo_99`. The
  server enforces this — the UI cannot opt out.

### Auth on loopback

**Default: on.** The launcher binds the REST sidecar on
`127.0.0.1:0` and generates a random per-boot bearer token. The
token reaches the browser tab via URL fragment (never sent to the
server, never logged) and is stashed in `sessionStorage`. See
§Security for the full mitigation set; this overrides the
loopback-default-off behaviour today's `codeless-cli serve` uses.

`--no-token` exists as an opt-out for trusted single-user systems
where the token prompt is friction; the launcher prints a warning
on every boot when set. The loopback bearer-gate bypass in
[`../codeless-cli/src/serve.rs`](../codeless-cli/src/serve.rs)
applies only to `codeless-cli serve`, not to the launcher.

## Security

The Tauri webview is unreachable from the network and unreachable
from other web origins. A browser tab on `127.0.0.1:<port>` is
neither. Replacing one with the other materially changes the trust
surface, and the doc has to say so.

**Default: `--require-token` is on.** The launcher generates a
random token on boot, writes it to `~/.codeless/launcher.url` with
mode `0600` (enforced via `OpenOptionsExt::mode`), and embeds it in
the first browser-open URL as a fragment:

```
http://127.0.0.1:<port>/<32-hex-prefix>/#token=<32-hex-token>
```

The fragment is **never sent to the server**, **never appears in
access logs**, and **never appears in the `Referer` header on
cross-origin requests**. The UI reads it from `window.location.hash`
on first load, stashes it in `sessionStorage` (per-tab,
not-`localStorage`), and clears the fragment via
`history.replaceState`. Every RPC call carries the token as a
bearer header.

This is the Jupyter pattern in full — the prior version cited
Jupyter for the prefix but skipped the token. Jupyter ships both
for a reason: the prefix alone falls open against any process on
the host that can read `~/.codeless/launcher.url` (NFS- or
tmpfs-mounted homes, shared dev boxes, container sidecars). The
token + `0600` closes that hole.

`--no-token` exists as an opt-out for trusted single-user systems
where the prompt is friction. It is **not** the default, and the
launcher prints a warning on every boot when set.

### DNS rebinding — `Host` header allowlist

A malicious site can set a short-TTL DNS record that resolves to
`127.0.0.1:<port>`, then make the user's browser send requests to
the launcher with the malicious site's origin. The launcher has no
token to reject them. The cheap, complete fix:

- Every REST route rejects any request whose `Host` header is not
  one of `127.0.0.1:<port>` or `localhost:<port>` (the actual bound
  port substituted at boot). Implemented as an Axum middleware on
  the same router `codeless-server` already serves.
- The reject is `421 Misdirected Request`, not `403` — semantic, and
  makes the failure mode obvious in logs.

### CORS lockdown — and what it does not catch

All cross-origin `fetch` / `EventSource` / `XMLHttpRequest`
requests are rejected. The UI is same-origin by construction; any
non-matching `Origin` header is refused.

**What CORS does not catch:** a `<form action="http://127.0.0.1:N/...">`
on a malicious page is a "simple request" — no preflight, no
`Origin` check on the server side. The defence against that is the
**Host header allowlist** above, not CORS: the malicious page
cannot spoof `Host` from a browser, so the request lands with
`Host: 127.0.0.1:N` (the actual loopback) and is rejected only if
the host check is enforced. If you read this section thinking CORS
is doing the work, it isn't — Host is. Both ship together; neither
is sufficient alone.

`codeless-server` does not set permissive CORS today; this is a
confirm-and-lock, not a new policy.

### No WebSocket / WebTransport upgrade endpoints

The transport is SSE over HTTP `GET`. The Host allowlist runs on
every request including the SSE start. If a future RPC adds a
WebSocket or WebTransport endpoint, the same Host-check middleware
**must run pre-upgrade** — Host on the upgrade request alone does
not gate frames after the upgrade completes. State this contract
in the RPC layer: no WS/WebTransport endpoints without an explicit
review of the Host enforcement path. The simplest way to keep this
true is to refuse to add such endpoints; the SSE-over-HTTP shape
already covers every subscribe pattern in use.

### Residual risks the launcher cannot fix

- **Malicious browser extensions on the user's profile** can read
  the URL bar (token + prefix), the `sessionStorage` content, or
  proxy requests. Today's Tauri webview has no such surface; the
  browser-tab model accepts this as a residual risk. Mitigation
  is on the user (audit installed extensions); the launcher
  cannot detect or prevent it.
- **Other processes on the host as the same UID** can read
  `~/.codeless/launcher.url` (it's `0600`, so other UIDs can't —
  multi-user shared boxes are safe). A compromised process
  running as the user already has full account access, so this
  is not a new exposure.
- **DevTools open on a browser tab** lets any code paste into the
  console run as the UI. Same trust model as any web app; named
  here so the §R5 rewrite below is honest.

### Random URL path prefix

Defence in depth: the launcher serves the entire app under a random
path prefix chosen at boot:

```
http://127.0.0.1:<port>/<32-hex>/
```

The prefix is written to `~/.codeless/launcher.url` alongside the
port (§Port-file). External tools needing `rest_url` read the
prefixed URL from `ServerInfo.rest_url`, which already exists. A
random website cannot guess the prefix; even if the Host check
fails open on some future config, the prefix is a second layer.

This is the Jupyter pattern — proven, cheap, low-friction. The
prefix rotates per launcher invocation; there is no "stable URL"
contract to break.

### Defaults summary

| Surface              | Default              | Override |
|----------------------|----------------------|----------|
| Loopback REST bind   | `127.0.0.1:0`        | `--bind <addr>` (CLI only) |
| Bearer token         | **on, random per boot** | `--no-token` (warns) |
| Token transport      | URL fragment → `sessionStorage` | — |
| `launcher.url` mode  | `0600`               | — |
| `Host` allowlist     | on, no opt-out       | — |
| CORS                 | same-origin only     | — |
| URL path prefix      | random per boot      | — |
| WS / WebTransport    | refused at the RPC layer | — |

The "no opt-out" rows are the defensible-loopback baseline. Without
them, the launcher is one DNS rebinding or one same-UID process
away from being a remote-code-execution surface for any browser tab
the user has open.

## RPC additions

### `EventFilter` collapses to a sum type

```rust
#[derive(Serialize, Deserialize, specta::Type)]
pub enum EventFilter {
    /// Events tagged with this repo only. The runtime publishes every
    /// job/stage/fs event with the owning `repo_id` already (see
    /// `codeless-runtime/src/rpc/workspaces.rs:117-128`); the fan-out
    /// filters at delivery time.
    Repo(RepoId),
    /// Library-level events only: `workspace_attached`,
    /// `workspace_detached`, `workspace_unhealthy`,
    /// `workspace_recovered`. Tab-agnostic — every tab that wants to
    /// keep its picker live opens one of these in addition to its
    /// `Repo` subscription.
    Library,
}
```

Same wire size as a struct with `repo_id: Option<RepoId>` + `scope:
EventScope`, but the invalid state ("`Repo` without a `repo_id`") is
unrepresentable. No `InvalidFilter` runtime error to write, no
`scope: Repo, repo_id: None` to test for. There is no "all repos"
mode; cross-workspace notification rides `Library`.

This is a breaking change to the wire format. Acceptable because
there are no external consumers yet — the only callers are the UI
in this repo. Land in the same change as the publish-site audit.

### `list_jobs` requires `repo_id`

Already an `Option<RepoId>` in `ListJobsArgs`. Flip the semantics:
`None` means "library view" (used by an admin / debug surface, not
the default UI flow), `Some(id)` is what tabs send. Document in
the typedoc that "None returns the union across the library —
calling code should pass `Some` unless it specifically wants
that."

### Other RPCs touched

Per the peer-review estimate, ~6 RPCs need `repo_id` plumbed
through. Final audit lands in the implementation job. Initial
candidates:

| Method | Change |
|---|---|
| `subscribe(EventFilter)` | `EventFilter` gains `repo_id` + `scope` |
| `list_jobs(ListJobsArgs)` | semantics tightened |
| `fs_cwd`, `fs_read_dir` (no path) | take `as_of_repo: RepoId` |
| `stop_active`, `gc_worktrees` | take `repo_id`; refuse `None` |
| `list_repos`, `list_workspaces` | unchanged; library-scope by design |

### Publish-site audit

Before estimating the work, audit every `EventBus::publish(...)`
call in `codeless-runtime` and confirm the published `Event` carries
a `repo_id` (or is a `Library`-scope event). The reviewer's "6
methods, 1 day" estimate holds iff the publish sites already tag
every event with `repo_id`. If they don't, that's the actual work.

## Launcher behaviour

### Boot sequence

1. Parse argv: `--workspace <path>` (optional, default `cwd`),
   `--native-window` (optional), `--no-browser` (optional, for
   headless dev — boots REST + tray but never opens a tab).
2. Boot `InProcessRpc` against the global
   `~/.codeless/codeless.sqlite`. The 2026-05-19 per-slug
   data-dir patch is reverted (see §Migration).
3. Bind the REST sidecar on `127.0.0.1:0`. The bound URL is the
   single source of truth for the UI; `ServerInfo.rest_url`
   already carries it.
4. If `--workspace <path>` is set, ensure the path is attached
   before opening the browser: look up the canonical path in
   `attached_workspaces`; if missing, run the same boot-upsert
   `--fs-root` already performs (canonicalise + `add_repo` +
   `attach_workspace`) so a `repo_id` exists. Then pass
   `?workspace=<repo_id>` on the browser URL so the tab hydrates
   `activeRepoId` immediately. If `--workspace` is unset, the tab
   lands on the workspace picker.
5. Spawn the system tray icon. Tray menu:
   `Open new tab` / `Open workspace…` / `Show running jobs` /
   `Quit`.
6. Open the user's default browser at the bound URL (suppress with
   `--no-browser`).
7. Wait for tray quit. On quit, drain in-flight jobs to next
   checkpoint, then exit cleanly.

### "Open browser" — Linux first

Linux is the only target this milestone proves. The launcher uses
`xdg-open <url>` with `$BROWSER <url>` as a fallback. Use the
`open` crate (or equivalent) so the macOS / Windows codepaths exist
in the same call site without separate plumbing — but their
behaviour is unverified until the follow-on platform milestone.

The launcher does not parse the browser's output; success is "spawn
returned 0", failure is "print the URL and continue running so the
user can copy-paste it".

| OS | Mechanism | Status |
|---|---|---|
| Linux | `xdg-open <url>`; fall back to `$BROWSER <url>` | in scope |
| macOS | `open <url>` | deferred — proves after Linux |
| Windows | `cmd /c start "" <url>` (empty title arg load-bearing) | deferred — proves after Linux |

### Lifecycle questions

- **What if the user closes every tab?** The launcher stays
  running in the tray. Tray menu's `Open new tab` re-opens at the
  bound URL.
- **What if the user clicks the tray icon when the launcher is
  already running?** Open a new tab. (macOS dock `Reopen` is
  expected to route through the same handler; verified in the
  follow-on platform milestone, not this one.)
- **What if Linux has no tray icon?** On GNOME without an indicator
  extension the tray is invisible; the launcher detects this at
  startup and logs a warning. The user's "open new tab"
  affordance becomes "re-run the binary" — the second invocation
  reads the port-file and opens a tab against the running
  launcher (see §Port-file). The Linux-no-tray UX is not a
  degraded mode; it is the documented fallback. `Quit` becomes
  `kill <pid>` against the PID stored in
  `~/.codeless/launcher.url`; the launcher catches `SIGTERM` and
  runs the same drain as the tray's `Quit` (defined below).

### Shutdown drain semantics

Tray `Quit` and `SIGTERM` are the same code path. Pinned:

1. Stop accepting new RPC subscribe / submit calls (`503` on the
   REST surface, `Unavailable` on the in-process RPC).
2. Send `Cancel` to every running job. The driver loop forwards
   to the runner, which closes its PTY / streams.
3. Wait up to **30 seconds** for `StageEnded` (or terminal status)
   on each cancelled job. Timer is per-launcher, not per-job —
   the whole drain is bounded at 30s, not 30s × N.
4. If the timer expires with jobs still running, log each
   non-terminated job ID with `level=warn target=shutdown.drain`
   and **force-exit with status 1**. The next launcher boot
   replays from the last checkpoint via the existing
   `job_driver_loop::replay_backlog` path.
5. If all jobs drain before the timer, exit status 0.

`SIGKILL` skips all of the above (kernel cannot run user code);
that's the user's choice and the replay path handles it. The 30s
timeout is a chosen number, not a measured one — revisit if dogfood
shows jobs that legitimately need longer to checkpoint.
- **What if the user runs the binary twice?** The second launch
  detects the port-file (see below), opens a new tab against the
  existing launcher's URL, and exits. This is single-instance,
  but only on the launcher process — there is no "multiple
  Tauri windows in one process" complexity because there are no
  Tauri windows.

### Port-file for second-launch detection

The launcher writes its bound URL + bearer token to
`~/.codeless/launcher.url` on boot. Format: two lines —
`url=http://127.0.0.1:<port>/<prefix>/` and `token=<32-hex>`. Mode
`0600`, written via tmp-file + rename for atomicity.

**Liveness via `flock(2)`, not PID-check.** The launcher takes an
**exclusive advisory `flock`** on the file for its lifetime
(`LOCK_EX | LOCK_NB`). A second launch tries the same lock
non-blocking; if it fails, the first launcher is alive, the
second reads the URL + token, opens a tab there, and exits. If the
lock succeeds, the first launcher is dead (the kernel released the
lock on process exit, even on `SIGKILL`) and the second launch
overwrites the file and becomes the launcher. No TOCTOU window
between liveness check and bind; no PID reuse race.

`flock` is Linux-only; this is a Linux-first milestone, so that's
fine. The macOS/Windows port-file strategy is a milestone-11
question (probably `fcntl(F_OFD_SETLK)` on macOS and
`LockFileEx` on Windows; not specified here).

This is simpler than `tauri-plugin-single-instance` because we
don't need IPC between processes — we just need "is there already
a launcher running, and where, and with what token."

## UI changes

The UI already imports only `RpcClient` (R2). The launcher serves
`HttpSseClient`-friendly endpoints (already there). Concrete
changes:

1. **Default `RpcClient` impl is `HttpSseClient`.** Today the
   desktop shell injects `TauriIpcClient`; flip this so the default
   path detects "am I in a Tauri webview?" and falls back to
   `HttpSseClient` if not. (The shell-detection helper in
   `src/lib/shell/` is the place.) `TauriIpcClient` also gets
   updated to send the new `EventFilter::Repo(_)` / `Library` shape
   on `subscribe` — when `--native-window` is in use the webview
   loads the same UI bundle, picks `TauriIpcClient`, and must
   speak the same wire as the browser-tab path. Don't let this
   silently break.
2. **`activeRepoId` plumbed into the subscribe call.** Every tab
   opens two subscriptions: `EventFilter::Repo(activeRepoId)` for
   workspace-scoped events and `EventFilter::Library` for picker
   updates.
3. **Deep-link is router-managed, not one-shot.** The router reads
   `?workspace=<repo_id>` from `window.location.search`
   **pre-hydration** — before any component mounts, before the
   workspace picker decides what to render. On every subsequent
   `setActive(repoId)`, call `history.replaceState` to keep the
   URL in sync. Refresh, share-link, and browser-back all land
   on the workspace the user was looking at. Verify in the M9
   exit test that a 50-workspace library does not race the
   `?workspace=` read.
4. **Tab-local state contract (hard rule, not audit-and-decide).**
   Cross-tab state leakage in `localStorage` is the new "shared
   SQLite" — same symptom, different layer. The contract:
   - Tab-local state (`activeRepoId`, scroll position, expanded
     panels, command palette history, recently-opened files,
     editor view state, AI chat draft state) lives **only** in
     zustand in-memory or `sessionStorage`. Never `localStorage`.
   - Library-level state (theme, keybindings, account-wide
     settings) goes through `RpcClient`, persisted server-side,
     not in browser storage.
   - Anything currently in `localStorage` that is tab-scoped
     either moves to `sessionStorage` or gets a `repo_id` key
     prefix and explicit per-tab read logic. No new
     `localStorage.setItem` without a written justification.
   This is enforced by lint, not by review: an ESLint rule (or
   equivalent grep in CI) refuses new `localStorage` references
   outside an allowlisted set of files. Land the rule in the
   same change as the existing-call-site cleanup.
5. **No new components.** No `<TabManager>`, no per-tab routing,
   no parallel UIs. Each browser tab is independent by virtue of
   being its own tab.

## Migration

### From 2026-05-19 per-workspace data dirs

A small boot shim:

1. On launcher boot, look for
   `~/.codeless/workspaces/*/codeless.sqlite`.
2. **Run the standard migration suite on each old DB before
   reading.** The schema in old per-slug DBs may have drifted by
   the time this shim runs (the 2026-05-19 patch landed yesterday
   but milestone 7 is weeks away). Open each old DB with the
   normal `InProcessRpc::with_file` path, which runs the migration
   chain, then read. If the old DB is at a schema version the
   shim's reader code doesn't support, log + skip + leave the dir
   in place; the user can re-attach by hand for that one.
3. `SELECT repo_id, fs_root_canonical, fs_root_display, attached_at
   FROM attached_workspaces` and the dependency rows from `repos`.
   `INSERT OR IGNORE` each row into the global
   `~/.codeless/codeless.sqlite` (also at the latest schema).
4. **Worktree path rewrite.** Old worktrees lived under
   `~/.codeless/workspaces/<slug>/worktrees/<job-id>`. The global
   DB's `jobs` rows reference those paths as absolutes. The shim
   walks the old `jobs` table, computes the new path
   (`~/.codeless/worktrees/<job-id>`), `mv`s the directory, and
   rewrites the stored path in the global DB to match. Job IDs
   are globally unique so collision is impossible; if the new
   path already exists, the shim refuses and surfaces an error
   (means the user has two old DBs claiming the same job ID —
   they have to pick one).
5. Rename the source dir to `<slug>.migrated` so a re-run is a
   no-op and the user can `rm -rf` if they wish.

This costs ~100 lines of Rust (not 50, given the worktree-path
rewrite) and saves the user from re-attaching every workspace they
created since the per-slug patch landed. The peer review
recommended writing this; this doc adopts that recommendation.

The pre-2026-05-19 `~/.local/share/codeless/codeless.sqlite` is
discarded by policy (predates the workspace concept; whatever
lives there is from CLI dogfood runs where re-attach is trivial).
The boot shim does not touch it.

**Schema-freeze the old shape now.** Capture the
`attached_workspaces` + `repos` + `jobs` schema at the 2026-05-19
patch's HEAD as a fixed `migration_v2_input.sql` test fixture; the
shim is tested against that fixture, not against a moving target.
The fixture lives in the implementation job's test data and
doesn't change once frozen.

## Cross-cutting rules (must hold)

- **R1**: no process spawn in the UI or in mobile-safe crates.
  The "open browser" call lives in `codeless-tauri-desktop`
  (host-only). The `open` crate stays in this crate.
- **R2**: the UI imports only `RpcClient`. The
  `TauriIpcClient` → `HttpSseClient` flip is in the shell
  injector, not in component code.
- **R3**: one responsive component. Tabs are not per-shell UIs —
  they are tabs in the same single-page app.
- **R4**: SQLite is the source of truth. The browser-tab model
  does not introduce client-side authoritative state.
- **R5**: single-tenant trust boundary is **preserved against web
  origins**: Host allowlist + CORS + same-origin UI + bearer token
  on by default (random per boot, in `0600` URL fragment) leave no
  network-reachable RPC path. New residual risks the Tauri webview
  did not have, named here so the doc is honest:
  - Malicious browser extensions on the user's profile can read
    the token from `sessionStorage` or proxy requests.
  - Other processes running as the same UID can read
    `launcher.url` — same-UID is full account access already, so
    not a new exposure, but stated explicitly.
  - DevTools opened on a tab can run console code as the UI.
  Multi-user hosts (different UIDs) are protected by the `0600`
  mode on `launcher.url`.

## Migration / backwards compat (CLI)

- `codeless-tauri-desktop` argv: existing `--workspace`,
  `CODELESS_WORKSPACE` env var both keep working. New:
  `--native-window`, `--no-browser`.
- `codeless-cli serve` is unchanged. The launcher's REST sidecar
  is the same `codeless-server` crate the CLI uses; both surface
  the same routes.
- `ServerInfo.rest_url` keeps its current contract — it's now
  the *primary* URL the UI talks to, not a secondary surface.

## Edge cases — explicit decisions

- **No default browser.** Launcher prints the URL to stderr and
  stays running. Tray icon menu offers "Copy URL to clipboard".
- **Default browser is on a different display / desktop.** The OS
  decides. Launcher does not try to move the window — that's the
  user's WM's job.
- **Two browsers in use.** The user's OS default is opened. The
  user can paste the URL into the other browser; tabs in different
  browsers are still independent clients of one launcher.
- **Browser blocks the `xdg-open` due to sandboxing.** Print the
  URL and continue. Tray menu offers "Open in browser" as a
  retry.
- **Wayland + sandboxed Firefox/Chrome (Snap, Flatpak).**
  `xdg-open` returns `0` (the desktop portal accepts the call),
  but the sandboxed browser may silently fail to open
  `127.0.0.1:<port>` URLs due to portal restrictions on local
  network access. The launcher cannot detect this — spawn-returned-0
  is the only signal it gets. Document: the success signal is
  spawn-returned-0; if no tab appears, the user copies the URL
  from the tray menu (or from stderr in headless mode). If this
  surfaces in dogfood, the mitigation is to print the URL
  unconditionally alongside the spawn, not to gate it on a
  "did the browser open?" check that doesn't exist.
- **User opens DevTools / saves as PWA.** Supported — it's just a
  web page on localhost. PWA-installability is a nice-to-have, not
  in scope for this milestone.
- **Headless server use.** `--no-browser` boots the launcher
  without opening anything; a remote user reaches it via
  `ssh -L <port>:127.0.0.1:<port>` and a browser on their own
  machine. The launcher does not need to know it's headless.

## Dogfood path — three decoupled projects

The doc as originally written bundled three projects together:

- **Project A — Fix the cross-talk bug.** The user-facing outcome.
  Server-side `repo_id` scoping + storage contract + UI passes
  `EventFilter::Repo(activeRepoId)` on every `subscribe`. Lands on
  the existing Tauri shell with one window. Dogfoodable as
  `codeless-cli serve --require-token` + two browser tabs against
  `http://127.0.0.1:N/#token=...`. **This is what gets "many
  repos at once" working.**
- **Project B — Launcher delivery.** Tauri shell becomes a tray-
  resident launcher that opens the user's browser. Revert
  per-slug data-dir; migration shim. Polish layer.
- **Project C — Harden public-loopback HTTP.** `codeless-server`
  gets Host allowlist, CORS lockdown, random prefix, fragment
  token. Required for B; not required for A (the existing
  `--require-token` flow is fine for dogfood between trusted
  tabs on the same machine).

**Shortest path to dogfooding "many repos at once":**

1. Land **M2** (publish-site audit). Tells you whether the
   `repo_id` scoping work is real or expanding.
2. Land **M3** (background-tab SSE cursor) + **M5** (`EventFilter`
   scoping) + **M6** (`list_jobs` + read RPC tightening). Server
   filters by repo at fan-out.
3. Land **M4** (tab-local storage contract). Block before any
   two-tab dogfood — otherwise `localStorage` leakage reintroduces
   the cross-talk symptom at the browser layer.
4. **Dogfood.** Run `codeless-cli serve --require-token`, open two
   browser tabs at the URL with the token, point each at a
   different attached workspace, work. No launcher, no tray, no
   migration, no §Security mitigations beyond the existing token
   gate (the dogfood tabs are trusted; the Host allowlist /
   fragment-token work is for the launcher distribution case).
   Validate the M4 contract under real load.
5. Then build **M7** (security hardening) + **M8–M10** (launcher
   + UI flip) as the polish layer that makes single-binary
   distribution work for non-developers.

The shortest path is days, not weeks. Bundling A + B + C is what
makes this a multi-week project; decoupling them gets the user
outcome first.

## Milestones

Status legend: `[x]` done, `[~]` partial, `[ ]` not started.

Linux is the platform that gates the design. macOS and Windows
ship after the Linux exit test (M10) lands green. Project tags
(A/B/C) match §"Dogfood path" — A is the dogfood-blocking set; A
done is "many repos at once" working.

1. `[ ]` **Decisions.** Confirm browser-launcher over
   single-instance; lock the migration approach; lock the
   security defaults from §Security. This doc captures the
   choice; the implementation job's stage 1 records the final
   answers.

   *Project A.*
2. `[ ]` **Publish-site audit.** Walk every
   `EventBus::publish(...)` in `codeless-runtime`; confirm each
   event carries a `repo_id` (or is library-scope). Land any
   missing `repo_id` fields on the event types before any
   filter logic depends on them. **This is the work that
   validates or invalidates the "6 RPCs, 1 day" estimate** —
   if events without a `repo_id` show up here, the scope
   expands and the rest of the milestones replan.

   *Project A. Gate on the whole plan.*
3. `[ ]` **Background-tab SSE cursor verification.** 10-line
   test: open a `subscribe` SSE connection, drop it, reconnect
   with the last `Since` cursor, assert no events missed and
   no duplicates. If the existing cursor doesn't survive
   disconnect, fix that **here**, not at M9 — it's a runtime
   bug regardless of the launcher.

   *Project A.*
4. `[~]` **Tab-local storage contract.** Land the §UI changes
   "hard rule" with the ESLint/CI grep. Move existing
   `localStorage` tab-scoped state to `sessionStorage` or
   in-memory zustand. Library state goes through `RpcClient`.
   **Blocks dogfood** — `localStorage` leakage reintroduces
   the cross-talk symptom at the browser layer.

   *Project A.* Partial: the audit and the tab-level moves
   landed under [`../../DOCS/STORAGE-AUDIT.md`](../../DOCS/STORAGE-AUDIT.md)
   (assistant focus thread, open job tabs → `sessionStorage`).
   The ESLint/CI grep that prevents future regressions is
   still open.
5. `[x]` **`EventFilter` sum-type + `subscribe` scoping.**
   Collapse `EventFilter` to `enum { Repo(RepoId), Library }`
   per §RPC additions. Server-side filter at fan-out. UI
   passes `EventFilter::Repo(activeRepoId)` on every
   `subscribe` and opens a parallel `Library` subscription.
   `TauriIpcClient` updated to the same wire.

   *Project A.*
6. `[x]` **`list_jobs` + other read RPCs.** Tighten semantics
   per §RPC additions. Audit table from §"Other RPCs touched"
   lands here.

   *Project A.* `fs_cwd`, `fs_read_dir`, `fs_read_file`,
   `fs_write_file` thread `repo_id`; the server resolves it
   to the attached workspace's `fs_root` via the shared
   `fs_root_for_repo` helper; calls with an unknown or
   detached `repo_id` are rejected.

   ---

   **Dogfood gate (A complete).** Run `codeless-cli serve
   --require-token`, open two browser tabs at the URL with
   the token, point each at a different attached workspace.
   Verify no cross-talk in events, jobs, files. Use the
   product like this for at least a week before starting
   Project B/C. If the M4 contract leaks anything tab-scoped,
   fix here, not after the launcher.

   ---

7. `[ ]` **Security hardening of `codeless-server`.** Land the
   §Security mitigations: `Host` allowlist middleware, CORS
   lockdown, random URL path prefix at boot, fragment-token
   delivery, `0600` on `launcher.url`. Tests: a request with
   `Host: evil.com` gets `421`; a `fetch` from a non-matching
   `Origin` is rejected; a request to the non-prefixed root
   path returns `404`. **Required for M8** — do not flip the
   launcher to a browser-tab UX without this.

   *Project C.*
8. `[ ]` **Revert per-workspace data-dir patch.** Restore
   global `~/.codeless/codeless.sqlite`. Write the migration
   shim (§Migration) including the worktree path rewrite.
   Delete the slug-derivation code in [`src/boot.rs`](./src/boot.rs).

   *Project B.*
9. `[ ]` **Launcher mode (Linux).** Tauri shell boots runtime
   + REST, opens browser via `xdg-open`, drops native webview,
   installs tray icon (with the headless-fallback path from
   §Lifecycle questions). Add `--native-window`, `--no-browser`,
   `--no-token` flags. `flock(2)` port-file second-launch
   detection. `SIGTERM` runs the drain pinned in §"Shutdown
   drain semantics".

   *Project B.*
10. `[~]` **UI shell-detection flip + deep-link router.**
    Default `RpcClient` impl becomes `HttpSseClient` when not
    in a Tauri webview. Read `?workspace=<repo_id>` from
    `window.location.search` **pre-hydration**;
    `history.replaceState` on every `setActive` to keep the
    URL in sync. Fragment-token bootstrap (read on first load,
    move to `sessionStorage`, clear from URL).

    *Project B + C.* Partial: the workspace-scoping job
    landed the deep-link router (`?workspace=<repo_id>`
    read pre-hydration, `history.replaceState` on every
    `setActive`). The shell-detection flip and the
    fragment-token bootstrap are still open and arrive with
    M7 + M9, where they belong.
11. `[ ]` **Linux exit test.** Two Firefox tabs against two
    different workspaces; submit a job in each; assert tab A's
    event stream does not contain tab B's `JobStarted` /
    `StageStarted` envelopes. Background one tab for 30s,
    foreground it, assert no missed events (validates M3
    under real conditions). Headless-launcher re-run opens a
    tab. `SIGTERM` to the launcher PID drains within 30s or
    force-exits with status 1.
12. `[ ]` **macOS + Windows.** Deferred until M11 is green.
    Covers: `open <url>` / `cmd /c start "" <url>`, macOS dock
    `Reopen` handler, Windows tray semantics, port-file lock
    mechanism (likely `F_OFD_SETLK` / `LockFileEx`),
    code-signing question (see §"Open questions" #3).

Project A (M1–M6) is the dogfood-blocking set. Project B + C
(M7–M11) is the polish layer that makes single-binary
distribution work for non-developers. M12 is platform expansion
gated on Linux proof.

**Exit tests.**
- M3: SSE reconnect with last `Since` cursor replays missed
  events with no duplicates.
- M5: `subscribe(EventFilter::Repo(r1))` does not receive
  events tagged `r2`, and vice versa. `EventFilter::Library`
  subscription receives `workspace_attached(r2)` while tab is
  scoped to `r1`.
- M7: `curl -H 'Host: evil.com'` → `421`; cross-origin `fetch`
  → rejected; root path without prefix → `404`; `stat
  ~/.codeless/launcher.url` shows mode `0600`.
- M8: migration shim test fixture — populate two
  `~/.codeless/workspaces/<slug>/codeless.sqlite` files
  matching `migration_v2_input.sql`, run boot, assert global
  DB has the union of attached rows, worktree dirs are moved,
  source dirs renamed `.migrated`.
- M9: launcher second-launch test — `cargo run` twice in
  series; assert second invocation exits cleanly and opens a
  tab against the first launcher's URL. Stale port-file with
  released `flock` is overwritten.
- M11: the user-facing exit criterion. If two tabs cross-talk,
  the milestone has not landed.

## Known issues

### Worktree root is not in the fs jail

Status: fixed in 168cb7f

Per-job worktrees live under `~/.codeless/worktrees/<job-id>` (CLI)
or `<workspace>/.codeless/worktrees/<job-id>` (desktop), both of
which sit outside the workspace root that seeds `HostFs` at boot.
Without an explicit `host_fs.add_root(&worktree_base)`, the
`agent_chat` RPC refuses any job whose `cwd` points into the
worktree base with `invalid_argument: agent_chat cwd is outside
the configured fs roots` (reject site:
`crates/codeless-runtime/src/rpc/chat.rs:76`).

- **Where it bites.** Opening the per-job chat panel for any
  running job whose worktree was provisioned by the runtime. Fix
  registers the worktree base with `HostFs` after
  `create_dir_all` and before `runtime.with_fs(...)` in both
  hosts: `crates/codeless-cli/src/serve.rs:413-422` (already on
  the branch before this job started) and
  `crates/codeless-tauri-desktop/src/boot.rs:161-169` (added in
  168cb7f). The two call sites share the same shape; surfacing
  errors uses `BootError::FsRoot` on the desktop side and the
  surrounding `anyhow` context on the CLI side.

## Open questions

1. **System tray on Linux.** Tauri 2's tray plugin works on
   `libayatana-appindicator` / `libappindicator3` distros. On
   GNOME without an indicator extension, the tray icon is
   invisible. The fallback UX is documented in §Lifecycle
   questions: re-run the binary to open a new tab, `kill <pid>`
   to quit. Confirm the kill path drains in-flight jobs the
   same way the tray `Quit` does during milestone 8.
2. **Headless dev loop.** `cargo tauri dev` currently opens the
   Tauri webview against Vite's dev server. Under launcher mode
   the dev loop becomes "cargo run + browser tab against
   `http://localhost:5173` proxying to the Rust REST port". The
   dev-loop docs (`README.md` in this crate) need updating; not
   in this scope.
3. **macOS code-signing for the launcher (deferred).** Resolves
   during milestone 11. Native-window Tauri apps need a
   notarised bundle; a tray-only launcher might be able to ship
   as a plain `.app` without webview entitlements — but only if
   the launcher genuinely stops embedding the webview runtime,
   which may require dropping `tauri::Builder` for a plain
   `tray-icon` binary. Determines whether milestone 11 is a
   Tauri refactor or a "extract launcher into a non-Tauri
   binary" job. Resolve before starting milestone 11, not
   during.
4. **`--native-window` deprecation timeline (resolved: never).**
   The flag stays indefinitely. Maintenance cost is near-zero
   (`TauriIpcClient` and the commands surface are already
   built); removing it later is easy, un-removing it is hard.
   Revisit only if the Tauri-IPC code surface starts blocking
   meaningful refactors.
5. **Web-platform file picker on Firefox/Safari.** The
   `showDirectoryPicker` fallback in WORKSPACE-ATTACH.md is a
   typed input. **Pre-milestone-8 experiment:** stand up the
   workspace-attach modal at `http://127.0.0.1:N/` for 30
   minutes and attach three workspaces in Firefox; if the
   typed-input feels equivalent to the OS picker the
   browser-default ships, if not the data informs whether
   `--native-window` needs to be more prominent on those
   browsers (or whether a Tauri-plugin path picker the browser
   tab can RPC into is worth building). The experiment is
   cheap; the answer determines whether the launcher's default
   UX is acceptable.
6. **Background-tab SSE throttling — promoted to M3.** Was an
   open question; now a milestone in its own right. Browsers
   throttle or suspend background tabs' JS and may close idle
   SSE connections; the cursor-replay path has to work. If
   the existing `subscribe` cursor doesn't survive disconnect,
   the fix is on the client side (cursor + reconnect loop)
   and the runtime — same code either way, doesn't change the
   doc.

## Known issues

### Worktree root is not in the fs jail — chat panel fails for any job

Status: open
Discovered: 2026-05-19 (dogfood, browser tab against
`codeless-cli serve` at `127.0.0.1:7777`)

**Symptom.** Opening the per-job chat panel against a job whose
worktree lives under `~/.codeless/worktrees/<job-id>` returns:

```
invalid_argument: agent_chat cwd is outside the configured fs roots:
/home/user/.codeless/worktrees/job-01KRYQJVK0G60MEZVFQ6KW3Y1F
```

**Root cause.** The `agent_chat` RPC validates `args.cwd` against
[`HostFs::is_path_allowed`](../codeless-adapters-host/src/fs.rs) and
also tolerates paths under any registered repo's `local_path`. The
chat panel passes the job's worktree path as `cwd`, but neither rule
covers it:

- the worktree root (`~/.codeless/worktrees/`) is **not** added to
  `HostFs` allowed roots when the server boots, and
- the worktree is not a registered repo's `local_path` — repos point
  at the user's source tree, not the per-job worktree directories.

The reject path in
[`codeless-runtime/src/rpc/chat.rs:75-78`](../codeless-runtime/src/rpc/chat.rs)
returns the error above.

**Where it bites.**
- `codeless-cli serve --worktree-root <X>`: `<X>` is never registered
  with `HostFs`. The CLI's `serve.rs` already builds the worktree
  manager from this value but does not call
  `host_fs.add_root(worktree_root)`.
- `codeless-tauri-desktop`: `boot.rs` derives
  `paths.worktree_base = <ws_dir>/worktrees`, passes it to
  `WorktreeManager::new`, and never adds it to `HostFs` either.

Both hosts have the same gap. Same fix applies to both.

**Fix.** Register the active worktree root with the `HostFs`
allowed-roots list at boot, alongside the existing rehydration of
`attached_workspaces` rows:

```rust
host_fs
    .add_root(&worktree_base)
    .map_err(|e| BootError::FsRoot(format!("{}: {e}", worktree_base.display())))?;
```

For `codeless-cli serve`, the equivalent edit lives in
[`crates/codeless-cli/src/serve.rs`](../codeless-cli/src/serve.rs)
next to the existing `attached_workspaces` rehydration block (line
~413, after the `--fs-root` and rehydrate loop both run). The CLI
already has `worktree_root_effective` resolved at that point — pass it
to `host_fs.add_root` once it exists on disk (the existing
`std::fs::create_dir_all(wt)` line just above guarantees that).

For the desktop, the same edit goes in
[`crates/codeless-tauri-desktop/src/boot.rs`](./src/boot.rs) inside
`boot()` after the `attached_workspaces` rehydration loop, before the
`runtime.with_fs(Arc::new(host_fs))` call. `paths.worktree_base` is
in scope.

**Why this is a real bug, not a config oversight.** The user never
attaches the worktree root themselves — it is an internal directory
owned by the runtime. Asking the user to `--fs-root
~/.codeless/worktrees` or to attach it via the UI would be exposing
an implementation detail. The runtime creates the directory, the
runtime uses the directory, the runtime should register the
directory.

**Scope clarification.** This is **not** caused by the per-workspace
data-dir patch (2026-05-19) or by the embedded-REST landing on
2026-05-19 — the same bug exists on `codeless-cli serve` with no
desktop involvement at all. The dogfood path that hits it (browser
tab against `codeless-cli serve`) just happens to be the cleanest
reproduction.

**Owner.** Anyone landing the next batch of fs.* hardening. Two-line
change in two crates; lands without milestone dependencies.

## References

- WORKSPACE-ATTACH TODO that this doc supersedes:
  [`../../DOCS/WORKSPACE-ATTACH.md`](../../DOCS/WORKSPACE-ATTACH.md)
  §"TODO — multi-window desktop isolation"
- UI architecture (the four-shell contract):
  [`../../DOCS/UI-ARCHITECTURE.md`](../../DOCS/UI-ARCHITECTURE.md)
- Project scope (R1–R5):
  [`../../DOCS/SCOPE.md`](../../DOCS/SCOPE.md)
- Workspace-level agent rules:
  [`../../CLAUDE.md`](../../../CLAUDE.md)
- Inner-repo agent rules:
  [`../../codeless/CLAUDE.md`](../../CLAUDE.md)
