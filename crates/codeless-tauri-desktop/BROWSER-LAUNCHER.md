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

Default: off. The launcher binds the REST sidecar on
`127.0.0.1:0`; the loopback check in
[`../codeless-cli/src/serve.rs`](../codeless-cli/src/serve.rs)
already disables the bearer gate for loopback binds. Browser tabs
on `http://127.0.0.1:<port>` connect with no token prompt.
`--require-token` stays as the opt-in for users who want the
token flow locally.

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
  drains in-flight jobs the same way the tray's `Quit` does.
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
2. For each found file, open it read-only, `SELECT repo_id,
   fs_root_canonical, fs_root_display, attached_at FROM
   attached_workspaces`, `INSERT OR IGNORE` each row into the
   global `~/.codeless/codeless.sqlite`. Same for the `repos`
   rows the attached rows depend on.
3. Rename the source dir to `<slug>.migrated` so a re-run is a
   no-op and the user can `rm -rf` if they wish.
4. Worktrees stay where they are; their job-id-keyed names are
   globally unique so cross-mapping is unnecessary.

This costs ~50 lines of Rust and saves the user from re-attaching
every workspace they created in the last day. The peer review
recommended writing this; this doc adopts that recommendation.

The pre-2026-05-19 `~/.local/share/codeless/codeless.sqlite` is
discarded by policy (predates the workspace concept; whatever lives
there is from CLI dogfood runs where re-attach is trivial). The
boot shim does not touch it.

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
- **User opens DevTools / saves as PWA.** Supported — it's just a
  web page on localhost. PWA-installability is a nice-to-have, not
  in scope for this milestone.
- **Headless server use.** `--no-browser` boots the launcher
  without opening anything; a remote user reaches it via
  `ssh -L <port>:127.0.0.1:<port>` and a browser on their own
  machine. The launcher does not need to know it's headless.

## Milestones

Status legend: `[x]` done, `[~]` partial, `[ ]` not started.

Linux is the platform that gates the design. macOS and Windows
ship after milestone 9 lands green on Linux.

1. `[ ]` **Decisions.** Confirm browser-launcher over
   single-instance; lock the migration approach; lock the
   security defaults from §Security. This doc captures the
   choice; the implementation job's stage 1 records the final
   answers.
2. `[ ]` **Publish-site audit.** Walk every
   `EventBus::publish(...)` in `codeless-runtime`; confirm each
   event carries a `repo_id` (or is library-scope). Land any
   missing `repo_id` fields on the event types before any
   filter logic depends on them. **This is the work that
   validates or invalidates the "6 RPCs, 1 day" estimate** —
   if events without a `repo_id` show up here, the scope
   expands and the rest of the milestones replan.
3. `[ ]` **Per-tab storage audit.** Grep the Terax-derived UI
   for `localStorage`, `sessionStorage`, `persist(`. For each
   hit, classify as library-level (keep) or tab-level (move to
   in-memory or `sessionStorage`). Land the moves. This blocks
   milestone 6 — cross-tab state leakage in `localStorage`
   reintroduces exactly the "two views share state" symptom
   the launcher is supposed to fix.
4. `[ ]` **Security hardening of `codeless-server`.** Land the
   §Security mitigations: `Host` allowlist middleware, CORS
   lockdown, random URL path prefix at boot. Tests: a request
   with `Host: evil.com` gets `421`; a `fetch` from a
   non-matching `Origin` is rejected; a request to the
   non-prefixed root path returns `404`. **Loopback
   default-off auth depends on this; do not flip the launcher
   default until this is green.**
5. `[ ]` **`EventFilter` + `subscribe` scoping.** Add
   `repo_id` + `EventScope` to `EventFilter`. Server-side
   filter at fan-out. UI passes `activeRepoId` on every
   `subscribe` and opens a parallel `Library` subscription for
   the picker.
6. `[ ]` **`list_jobs` + other read RPCs.** Tighten semantics
   per §RPC additions. Audit table from §"Other RPCs touched"
   lands here.
7. `[ ]` **Revert per-workspace data-dir patch.** Restore
   global `~/.codeless/codeless.sqlite`. Write the migration
   shim (§Migration). Delete the slug-derivation code in
   [`src/boot.rs`](./src/boot.rs).
8. `[ ]` **Launcher mode (Linux).** Tauri shell boots runtime
   + REST, opens browser via `xdg-open`, drops native webview,
   installs tray icon (with the headless-fallback path from
   §Lifecycle questions). Add `--native-window` and
   `--no-browser` flags. Port-file second-launch detection.
   `SIGTERM` drains in-flight jobs the same way tray-`Quit`
   does.
9. `[ ]` **UI shell-detection flip + deep-link router.**
   Default `RpcClient` impl becomes `HttpSseClient` when not
   in a Tauri webview. Read `?workspace=<repo_id>` on initial
   load; `history.replaceState` on every `setActive` to keep
   the URL in sync. Background-tab reconnect: when the SSE
   channel closes (browser throttles backgrounded tabs), the
   client replays from the last `Since` cursor — verify this
   already works against `subscribe`'s existing cursor
   support.
10. `[ ]` **Linux exit test.** Two Firefox tabs against two
    different workspaces; submit a job in each; assert tab A's
    event stream does not contain tab B's `JobStarted` /
    `StageStarted` envelopes. Background one tab for 30s,
    foreground it, assert no missed events. Headless-launcher
    re-run opens a tab. `SIGTERM` to the launcher PID drains
    cleanly.
11. `[ ]` **macOS + Windows.** Deferred until milestone 10 is
    green. Covers: `open <url>` / `cmd /c start "" <url>`,
    macOS dock `Reopen` handler, Windows tray semantics,
    code-signing question (see §"Open questions" #3).

Milestones 1–10 are the Linux-proven path. Each is
independently shippable; the gates are stated above (storage
audit blocks 8; security blocks default-off auth; publish-site
audit replans 5–6 if events lack `repo_id`).

**Exit tests.**
- Milestone 4: `curl -H 'Host: evil.com'` → `421`;
  cross-origin `fetch` → rejected; root path without prefix →
  `404`.
- Milestone 5: `subscribe(EventFilter { repo_id: Some(r1),
  scope: Repo })` does not receive events tagged `r2`, and
  vice versa. Library-scope subscription receives
  `workspace_attached(r2)` while tab is scoped to `r1`.
- Milestone 7: migration shim test fixture — populate two
  `~/.codeless/workspaces/<slug>/codeless.sqlite` files, run
  boot, assert global DB has the union of attached rows and
  the source dirs are renamed `.migrated`.
- Milestone 8: launcher second-launch test — `cargo run`
  twice in series; assert second invocation exits cleanly and
  opens a tab against the first launcher's URL. Stale
  port-file with dead PID is overwritten.
- Milestone 10: the user-facing exit criterion. If two tabs
  cross-talk, the milestone has not landed.

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
6. **Background-tab SSE throttling.** Browsers throttle or
   suspend background tabs' JS and may close idle SSE
   connections. The UI must replay from the last `Since`
   cursor on reconnect; the existing `subscribe` cursor
   support should cover this. Verify during milestone 9 — if
   it doesn't, the fix is on the client side (cursor +
   reconnect loop), not the server.

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
