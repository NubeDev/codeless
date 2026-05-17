# MANI — Codeless multi-repo workflow

This workspace uses [`mani`](https://github.com/alajmo/mani) to orchestrate
work across the colocated repos. The bundled binary lives at
[`bin/mani`](../bin/mani) and is the canonical version for this workspace —
do not assume `which mani` resolves to the same build.

The config is [`mani.yaml`](../mani.yaml) at the workspace root.

## TL;DR — every command from the workspace root

```bash
cd /home/user/code/rust/codeless-workspace

# Status across both repos (codeless + ai-runner)
./bin/mani --config mani.yaml run status --all

# Codeless-only commands (the common case for JOB-LOOP ticks)
./bin/mani --config mani.yaml run status --projects codeless
./bin/mani --config mani.yaml run diff --projects codeless

# Commit + push (used by every JOB-LOOP tick)
MSG='stage N: <title>' \
  ./bin/mani --config mani.yaml run commit --projects codeless
./bin/mani --config mani.yaml run push --projects codeless
```

## Why this and not plain git

JOB-LOOP ticks are scripted, fired by a fresh agent every minute. Plain
git invocations need a cwd; mani invocations specify the project by
name and run from the workspace root. That removes a class of "wrong cwd
→ wrong repo" errors when ticks juggle multiple repos in the same batch.

The workspace also bundles its own mani binary (`bin/mani`) so a fresh
clone runs the same version regardless of what the host has on `$PATH`.

## Project map

| Project     | Path        | Tags                      | Notes                                                                       |
|-------------|-------------|---------------------------|-----------------------------------------------------------------------------|
| `codeless`  | `codeless`  | `rust`, `active`          | The main repo (`NubeDev/codeless`). Independent GitHub remote.              |
| `ai-runner` | `ai-runner` | `rust`, `vendored`, `reference` | Vendored from `rubix-agent/crates/ai-runner`. No `.git`. Manual re-vendor.  |

`terax-ai` is **not** a project in this workspace — it lives at
`~/code/rust/terax-ai/` as an external read-only reference for the UI port.

## Tasks reference

All tasks read from [`mani.yaml`](../mani.yaml).

| Task     | What it does                                              | JOB-LOOP usage                          |
|----------|-----------------------------------------------------------|------------------------------------------|
| `status` | `git status --short --branch`                             | Tick pre-flight (clean tree check)      |
| `fetch`  | `git fetch --all --prune`                                 | Occasional sync; not per-tick           |
| `pull`   | `git pull --ff-only`                                      | When upstream has progressed            |
| `branch` | Current branch + ahead/behind upstream counts             | Verify push succeeded (ahead-by-0)      |
| `diff`   | `git diff --stat`                                         | Inspect a stage's changes pre-commit    |
| `commit` | Stage + commit with `$MSG` (env var, **required**)        | Every stage of every tick               |
| `push`   | Push current branch, set upstream if needed               | Every stage of every tick               |

The `commit` task **refuses** to run with an empty `MSG`. JOB-LOOP ticks
must set it explicitly:

```bash
MSG='stage 8: cargo workspace + crate stubs per SCOPE' \
  ./bin/mani --config mani.yaml run commit --projects codeless
```

## Filtering by tag (less common but useful)

```bash
# Run a task only on Rust projects
./bin/mani --config mani.yaml run status --tags rust

# Skip the vendored ones
./bin/mani --config mani.yaml run status --tags active
```

## Custom mani commands in this build

The bundled `bin/mani` is a custom build, not stock upstream. Extra
commands available:

- `release` — orchestrates cargo/npm release flow. Out of scope until
  Codeless ships its first release.
- `issue` — manages GitHub issues across repos. Useful when a JOB-LOOP
  tick discovers something worth tracking.
- `introspect` — prints the full command tree as JSON, designed for
  LLM consumption. Useful when bootstrapping a new agent on this
  workspace.
- `check` — validates `mani.yaml` parses and references resolve. Run
  after editing the config.

## Footguns

- **`run status --all` walks into `ai-runner/` which has no `.git`.**
  git silently walks up the parent chain and reports the *workspace*
  state. Result: `ai-runner` rows in `mani run status --all` may show
  unrelated workspace modifications. Workaround: prefer
  `--projects codeless` for JOB-LOOP ticks.
- **Plain `mani` (system `$PATH`)** may be a different version with a
  different YAML schema. Always invoke as `./bin/mani --config mani.yaml`.
- **Don't `git add` the `codeless/` directory inside the workspace.**
  git will offer to add it as a gitlink (submodule). The workspace
  `.gitignore` already excludes it; the inner repo manages itself.

## When to add a new task

Add a task to `mani.yaml` when it's:

1. Run more than once across a tick (so scripts can shorten),
2. Cross-repo coordination glue,
3. Something a JOB-LOOP guardrail explicitly references.

Don't add a task for one-off ad-hoc commands — they belong in the
session doc's Notes.
