# SETUP — building codeless from a fresh clone

This is the build-from-zero companion to
[`GETTING-STARTED.md`](./GETTING-STARTED.md). That doc assumes the
workspace already compiles and walks you through the runtime; this
one covers the **prerequisites and sibling repos** you need on disk
before `cargo run` will succeed.

If you just want to start an already-built server, skip to
[`GETTING-STARTED.md`](./GETTING-STARTED.md).

## Directory expectations

The Rust workspace path-deps a sibling repo (`ai-ui`) that lives
**one level above** `codeless-workspace/`. Your layout must look
like:

```
~/code/rust/
├── ai-ui/                       ← NubeDev/ai-ui, sibling of codeless-workspace
└── codeless-workspace/
    ├── ai-runner/               ← vendored (no .git of its own)
    └── codeless/                ← this repo (NubeDev/codeless)
```

The path-deps in [`crates/codeless-server/Cargo.toml`](../crates/codeless-server/Cargo.toml)
resolve `ai-ui-core`, `ai-ui-types`, `ai-ui-prompt` via
`../../../../ai-ui/crates/<name>`. If `ai-ui/` is missing or not at
that relative location, `cargo` fails at manifest-load time with
"failed to read `…/ai-ui/crates/ai-ui-core/Cargo.toml`".

`CODELESS_HOME` (default `~/.codeless`) for runtime state is
unrelated to where the source lives — see GETTING-STARTED §Layout.

## Prerequisites

| tool      | version            | why                                                                 |
|-----------|--------------------|---------------------------------------------------------------------|
| Rust      | MSRV 1.78+         | workspace `rust-version`; clippy `-D warnings` is non-negotiable    |
| pnpm      | 10.x               | UI uses `pnpm-lock.yaml` and a pnpm workspace under `ui/codeless-ui`|
| Node      | 22.x (Vite 7)      | matches the UI's lockfile                                           |
| git       | 2.40+              | per-job `git worktree` is the isolation primitive                   |
| `claude`  | optional           | enables the Claude CLI runner; otherwise pass `CODELESS_RUNNERS=mock`|

Linux only: bump the inotify watch limit, or Vite trips on it.

```sh
sudo sysctl fs.inotify.max_user_watches=524288
# persist:
echo 'fs.inotify.max_user_watches=524288' | sudo tee /etc/sysctl.d/90-inotify.conf
```

## One-time clones

```sh
mkdir -p ~/code/rust && cd ~/code/rust

# the sibling ai-ui workspace (path-dep'd by codeless-server)
git clone https://github.com/NubeDev/ai-ui.git

# the outer workspace + inner codeless repo
git clone https://github.com/NubeDev/codeless-workspace.git
cd codeless-workspace
git clone https://github.com/NubeDev/codeless.git
```

`codeless-workspace/` already ships `ai-runner/` vendored in-tree —
do not clone rubix-agent yourself. Patches applied to it are logged
in [`../../ai-runner.PATCHES.md`](../../ai-runner.PATCHES.md).

## Build

From inside `codeless-workspace/codeless/`:

```sh
# rust workspace — compiles every crate, runs unit + integration tests
cargo test --workspace

# lint gate (matches CI)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# UI deps — pnpm workspace with internal package @codeless/plugin-ui-sdk
pnpm -C ui/codeless-ui install
```

The first `cargo` build is slow (full dependency compile, including
the path-dep'd `ai-ui` crates and `ai-runner`). Subsequent runs use
the local cache.

If you forget `pnpm install`, Vite fails with
`Failed to resolve import "@codeless/plugin-ui-sdk"` because the
internal workspace package lives at
[`ui/codeless-ui/packages/plugin-ui-sdk/`](../ui/codeless-ui/packages/plugin-ui-sdk/)
and is only linked into `node_modules/@codeless/` by an install pass.

## Verify the build

```sh
# server binary should resolve
cargo run --quiet -p codeless-cli --bin codeless -- --help

# UI dev server should start on http://127.0.0.1:1420
pnpm -C ui/codeless-ui dev
```

If both succeed, hand off to
[`GETTING-STARTED.md`](./GETTING-STARTED.md) for the per-user runtime
layout (`~/.codeless/`), the `init-session.sh` wrapper, and
registering a repo via RPC.

## Common build failures

- **`failed to read .../ai-ui/crates/ai-ui-core/Cargo.toml`** — the
  sibling `ai-ui` repo is missing or not at
  `~/code/rust/ai-ui/`. Clone it (above) at exactly that relative
  position; the path-dep is `../../../../ai-ui` from inside
  `crates/codeless-server/`.
- **`Failed to resolve import "@codeless/plugin-ui-sdk"`** — UI
  dependencies were never installed. Run
  `pnpm -C ui/codeless-ui install`.
- **`Address already in use` on 7777** — a prior server is still
  bound. `./setup/init-session.sh stop` (or `fuser -k 7777/tcp`).
- **Vite EMFILE / inotify errors on Linux** — raise
  `fs.inotify.max_user_watches` (see Prerequisites).
- **`claude: command not found` at server start** — only matters if
  you set `CODELESS_RUNNERS=claude` (the default). Either install
  the Claude CLI on `PATH`, point `CLAUDE_BINARY` at it, or run with
  `CODELESS_RUNNERS=mock` for a non-execution server.

## Pointers

- Runtime layout + the `init-session.sh` wrapper:
  [`GETTING-STARTED.md`](./GETTING-STARTED.md)
- Submitting a first job (worktree mode, branch naming):
  [`ADDING-JOB.md`](./ADDING-JOB.md)
- Per-repo project memory: [`../CODELESS.md`](../CODELESS.md)
- Workspace-wide agent rules: [`../../CLAUDE.md`](../../CLAUDE.md)
- Full project scope: [`../../DOCS/SCOPE.md`](../../DOCS/SCOPE.md)
