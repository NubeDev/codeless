# START-SERVER-UI — running the codeless server + UI locally

Two terminals, one browser. Terminal A runs the Rust core
(`codeless-server`); terminal B runs the Vite dev server for the
React UI. The browser loads the UI from `:1420`, the UI talks RPC +
SSE to the core on `:7777`.

This is the local-mode flow used by the demo and the per-tick UX
sessions. Hosted-mode auth (Phase 7) and the desktop / mobile shells
are not in scope here — see [`SCOPE.md`](./SCOPE.md).

## Prerequisites

- Rust toolchain at the workspace MSRV (1.78).
- `pnpm` (UI workspace; do not substitute `npm`/`yarn` — lockfile is `pnpm-lock.yaml`).
- Linux only: inotify watcher limit high enough for Vite. If you hit
  `ENOSPC: System limit for number of file watchers reached`:
  ```sh
  sudo sysctl fs.inotify.max_user_watches=524288
  ```
  Persist by adding `fs.inotify.max_user_watches=524288` to
  `/etc/sysctl.conf`.

## Terminal A — codeless server (port 7777)

```sh
cd /home/user/code/rust/codeless-workspace/codeless

# 1. Seed the demo DB (idempotent — safe to re-run).
cargo run -p codeless-cli --bin codeless -- \
  --db /tmp/codeless-demo.db demo bootstrap

# 2. Start the server bound to loopback. Keep job worktrees OUT of the
#    source tree so they don't pollute `git status` or trip Vite's
#    file watcher.
cargo run -p codeless-cli --bin codeless -- \
  --db /tmp/codeless-demo.db serve \
  --bind 127.0.0.1:7777 \
  --fs-root "$PWD" \
  --worktree-root /tmp/codeless-worktrees
```

What the flags do:

| Flag | Why |
|---|---|
| `--db /tmp/codeless-demo.db` | SQLite source of truth (R4). The same path must be passed to every `codeless` invocation that should see the same jobs. `:memory:` is useless across processes. |
| `--bind 127.0.0.1:7777` | Loopback is unauthenticated by design — the trust boundary is same-user same-host (R5). Anything other than `127.0.0.1` / `::1` requires `--require-token`. |
| `--fs-root "$PWD"` | Root the `fs.*` RPC surface is allowed to read/write under. Without it, the editor surfaces return `Internal`. The server canonicalises every `fs_*` path and rejects anything outside this root. |
| `--worktree-root /tmp/codeless-worktrees` | Where per-job `git worktree` checkouts land (`<root>/job-<id>` on a fresh `codeless/job-<id>` branch). **Always set this explicitly to a path outside the source tree.** If unset, it defaults to `<fs-root>/.codeless/worktrees` — i.e. dumped inside the repo you're editing, where every job creates a fresh checkout that shows up in `git status` of the workspace and gets picked up by Vite's file watcher. |

### Where to put `--worktree-root`

Pick a path **outside `--fs-root`**. Anywhere writable works; the
directory is created on demand. Suggestions:

- `/tmp/codeless-worktrees` — ephemeral; cleared on reboot. Good for
  the demo and short sessions.
- `~/.cache/codeless/worktrees` — survives reboot, scoped to the
  user. Good for ongoing development.
- `$CODELESS_WORKTREE_ROOT` — same flag, env-var form, for when you
  don't want to repeat it on every `serve` invocation.

Avoid:

- Anywhere under `--fs-root` (the source tree). Per-job checkouts
  there will pollute `git status`, get indexed by your editor, and
  re-trigger Vite HMR on every job tick.
- `/tmp` itself (use a subdirectory) — keeps cleanup simple.

If you already have stale worktrees from a previous run with the
default path, clean them up with:

```sh
git -C /home/user/code/rust/codeless-workspace/codeless worktree prune
rm -rf /home/user/code/rust/codeless-workspace/codeless/.codeless/worktrees
```

`git worktree prune` first so git forgets the registrations; then
the directory can be removed.

Optional add-ons (off by default — the binaries / keys may not be
installed):

- `--enable-claude` — uses the `claude` binary on `PATH` (or
  `CLAUDE_BINARY`).
- `--enable-anthropic` — reads `anthropic_api_key` from the secrets
  file.

The server logs to stdout. Leave it running. Submitted jobs are driven
by the in-process background driver (concurrency 4; tune with
`--driver-concurrency`).

## Terminal B — UI dev server (port 1420)

```sh
pnpm -C /home/user/code/rust/codeless-workspace/codeless/ui/codeless-ui dev
```

Vite serves on `http://127.0.0.1:1420`. The port is pinned in
[`vite.config.ts`](../codeless/ui/codeless-ui/vite.config.ts) — it is
**not** the Vite default `5173`. HMR is enabled; most edits do not
need a restart. Hard-restart Vite only when changing
`vite.config.ts`, env files, or installed deps.

## Browser

Open `http://127.0.0.1:1420`. The browser shell injects an
`HttpSseClient` (see [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md))
pointing at `http://127.0.0.1:7777`. SSE handles the live event
stream; REST POST handles RPC method calls.

## Resetting state

Stop the server, delete the DB, re-bootstrap:

```sh
rm /tmp/codeless-demo.db
cargo run -p codeless-cli --bin codeless -- \
  --db /tmp/codeless-demo.db demo bootstrap
```

The UI does not need a restart for this — it will reconnect SSE and
re-fetch on the next interaction.

## Troubleshooting

- **UI stuck on `loading…` / `waiting for events…` after clicking
  through several jobs.** SSE connection leak — should be fixed in
  [`hooks.ts`](../codeless/ui/codeless-ui/src/lib/rpc/hooks.ts). If
  it recurs, check that `useEventStream` / `useReviews` call
  `iter.return()` in cleanup. Chrome caps SSE at 6 connections per
  origin; a leak past that stalls every new subscription.
- **`address already in use` on 7777 or 1420.** Stale process:
  `ss -ltnp | grep -E ':(7777|1420)'` then `kill` the pid.
- **`ENOSPC` from Vite.** Inotify limit — see prerequisites.
- **`fs_*` methods return `Internal`.** Server was started without
  `--fs-root`. Restart it with the flag pointed at the repo root.
- **Jobs stay `Queued` forever.** Either `--no-driver` was passed, or
  the runner the job requested isn't enabled (`--enable-claude` /
  `--enable-anthropic`) and didn't fall back to `mock`.
