# GETTING-STARTED — local codeless setup

This is the **first thing to read** if you've just cloned the repo
and want a working local server with the React UI in the browser.

It documents the per-user layout (`~/.codeless/`), the helper script
that wraps the bare `cargo run …` invocations, and how to add a repo.

For the deeper docs see:

- [`DOCS/SCOPE.md`](../DOCS/SCOPE.md) — overall architecture (R5: single trust boundary, many concurrent jobs, one SQLite DB).
- [`DOCS/START-SERVER-UI.md`](../DOCS/START-SERVER-UI.md) — raw flag-by-flag invocation; what this script wraps.
- [`DOCS/HACKLINE-DEV.md`](../DOCS/HACKLINE-DEV.md) — pointing the server at a target repo + branch control per job.

## Layout

The wrapper assumes a single per-user root with sub-buckets for the
things codeless owns:

```
~/.codeless/                  ← override with $CODELESS_HOME
├── codeless.db               single SQLite, all repos, all jobs
├── worktrees/                per-job git worktree checkouts
│   └── job-<ULID>/
├── logs/server.log           tail this when running --bg
└── server.pid                pid file for `init-session.sh stop`
```

**Why one DB across all repos?** SCOPE.md R4 makes SQLite the source
of truth and R5 makes the server single-tenant: one user, one trust
boundary, many concurrent jobs across many repos. Per-repo DBs would
fight that architecture and force one server process per repo.

**Secrets stay where they are** — `~/.config/codeless/secrets.toml`
(XDG default the CLI already uses for `secrets get/set` and the
bearer token). The wrapper does not move them.

### Cross-platform note

The `~/.codeless/` convention is the old-Unix style (think `~/.ssh`,
`~/.aws`). It's intentionally simple and works identically on Linux,
macOS, and Windows (under Git Bash / WSL). The pure-XDG alternative
would split state across `$XDG_DATA_HOME`, `$XDG_CONFIG_HOME`, and
`$XDG_CACHE_HOME`; that's more correct on Linux but harder to reason
about and doesn't survive `cp -r ~/.codeless backup/` cleanly. If a
future codeless deployment needs strict XDG, override `CODELESS_HOME`.

## One-time prerequisites

- Rust toolchain at the workspace MSRV (1.78).
- `pnpm` (the UI uses `pnpm-lock.yaml`).
- `claude` binary on `PATH` (or `CLAUDE_BINARY`) if you want the
  Claude runner enabled.
- Linux only: bump `fs.inotify.max_user_watches=524288` so Vite
  doesn't trip on the file-watcher limit.

## The wrapper: `setup/init-session.sh`

All paths are derived from `CODELESS_HOME` (defaults to
`~/.codeless`). No need to remember `--db`, `--worktree-root`, or
`--fs-root` flags between sessions.

```sh
# show resolved paths without doing anything
./setup/init-session.sh paths

# start in foreground (Ctrl-C to stop)
./setup/init-session.sh start

# start in background; logs to ~/.codeless/logs/server.log
./setup/init-session.sh start --bg
./setup/init-session.sh status
./setup/init-session.sh stop

# wipe everything under ~/.codeless (asks for confirmation)
./setup/init-session.sh reset
```

### Tweaks via env vars

| var | default | effect |
|---|---|---|
| `CODELESS_HOME` | `~/.codeless` | root for db, worktrees, logs, pid |
| `CODELESS_BIND` | `127.0.0.1:7777` | server bind addr |
| `CODELESS_FS_ROOT` | unset | passes `--fs-root`; see below |
| `CODELESS_RUNNERS` | `claude` | comma list: `claude,anthropic` (or `mock` to disable both) |
| `CODELESS_DRIVER_CONCURRENCY` | `4` | parallel jobs in the background driver |

### About `--fs-root`

`--fs-root` is the **security boundary** for the `fs.*` RPC surface
(editor read/write). It is server-wide today, not per-repo, so set it
to whichever repo you're actively editing through the UI:

```sh
CODELESS_FS_ROOT="$HOME/code/rust/codeless-workspace/codeless" \
  ./setup/init-session.sh start
```

If unset, `fs_*` RPC methods return `Internal` and the editor pane in
the UI can't read files. The job runner itself (worktrees,
`add_repo`, etc.) does **not** require `--fs-root`.

## Adding a repo

Once the server is running, register the repo you want jobs to
target. The wrapper has a thin convenience for the common case:

```sh
./setup/init-session.sh add-repo codeless \
  /home/user/code/rust/codeless-workspace/codeless

./setup/init-session.sh list-repos
```

That hits `POST /rpc/add_repo` with sensible defaults
(`default_branch=master`, `default_runner=claude`,
`git_auth=token:GITHUB_TOKEN`). For non-defaults, call the RPC by
hand — see [`DOCS/HACKLINE-DEV.md`](../DOCS/HACKLINE-DEV.md) for the
JSON shape and the full set of `SubmitJobArgs` knobs (workspace_mode,
branch, cost cap, etc.).

## Then the UI

In a second terminal:

```sh
pnpm -C ui/codeless-ui dev
# → http://127.0.0.1:1420
```

Open `http://127.0.0.1:1420`, the registered repo appears in the
sidebar, click **new job**.

## Common gotchas

- **"still says hackline"** — old DB has it. `./setup/init-session.sh
  reset`, then `start`, then `add-repo` your real repo.
- **`Address already in use`** — a previous server is still bound.
  `./setup/init-session.sh stop`, or `fuser -k 7777/tcp`.
- **Editor pane returns `Internal`** — `--fs-root` is unset. Set
  `CODELESS_FS_ROOT` and restart.
- **Worktrees polluting your source tree** — the wrapper always sets
  `--worktree-root` to `~/.codeless/worktrees/`, so this should not
  happen. If it does, you started the server without the wrapper.
- **Stale worktrees after `reset`** — git still has them registered
  in the source repo. Run `git -C <repo> worktree prune`.
