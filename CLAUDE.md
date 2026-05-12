# CLAUDE.md — rules for agents working inside the codeless repo

You are working inside `codeless/`, the inner Rust workspace for the
Codeless project. The outer multi-repo workspace
[`codeless-workspace`](https://github.com/NubeDev/codeless-workspace)
owns the design docs and shared tooling. Read this file first; the
durable per-repo memory is in [`CODELESS.md`](./CODELESS.md); full
project scope is in [`../DOCS/SCOPE.md`](../DOCS/SCOPE.md); the
agent-rule contract that supersedes anything ambiguous here is
[`../CLAUDE.md`](../CLAUDE.md).

If any rule below contradicts the workspace `../CLAUDE.md` or
`../DOCS/SCOPE.md`, **those win** — fix this file rather than diverge.

## Repository orientation

```
codeless/                  ← this repo (NubeDev/codeless)
├── Cargo.toml             ← workspace root; member list pins the crate layout
├── CLAUDE.md              ← this file
├── CODELESS.md            ← durable per-repo memory
├── README.md
└── crates/
    ├── codeless-types/             # wire types — iOS-safe, Android-safe
    ├── codeless-rpc/               # RpcServer trait + arg/result types — iOS-safe, Android-safe
    ├── codeless-runtime/           # state machine, sqlx, event bus — host-only
    ├── codeless-adapters-host/     # worktree, PTY, secrets file, process spawn — host-only
    ├── codeless-server/            # axum + SSE — host-only
    ├── codeless-client/            # HttpSseClient (RpcClient impl) — iOS-safe, Android-safe
    ├── codeless-cli/               # the `codeless` binary — host-only
    └── codeless-tauri-desktop/     # desktop shell — host-only
```

There is also a sibling vendored `ai-runner/` crate **outside this repo**
in the workspace. Treat it as read-only; updates flow from the upstream
rubix-agent workspace.

## Hard rules — trip one and the loop halts

### R1 — Crate dependency direction is enforceable

The iOS-safe / Android-safe columns in
[`../DOCS/SCOPE.md`](../DOCS/SCOPE.md#crate-layout-load-bearing-not-aspirational)
define which crates may reach which other crates. The mobile shell
(`codeless-tauri-mobile`, Phase 6) compiles only `codeless-types` and
`codeless-client`.

- **Never** import `std::process` or `tokio::process` from any crate
  other than `codeless-adapters-host`. Process spawning lives there and
  there only. A grep of the source tree for `process::Command` outside
  that crate must return zero matches.
- **Never** add a dependency from a mobile-safe crate (`-types`,
  `-rpc`, `-client`) onto a host-only crate (`-runtime`, `-adapters-
  host`, `-server`, `-cli`, `-tauri-desktop`).
- Host-only crates may freely depend on mobile-safe crates. The
  asymmetry is the whole point of the layering.

### R2 — Comments explain *why*, never *what*

Comments are how the next AI agent and the next human understand
intent. The code already says what it does.

- **No emojis.** Anywhere. Ever.
- **No task-status comments.** Never reference stages, ticks, tickets,
  "added in stage 3", "TODO from M5", "fixed for PR #123". The comment
  must still make sense after the loop finishes and the branch merges.
- **No restatements.** `// increment counter` above `counter += 1` is
  noise. Delete it.
- **No decorative banners, dividers, or ASCII art.**
- **No multi-paragraph essays.** If you need three paragraphs, the
  design is wrong — fix the code or move the explanation into
  `../DOCS/`.

The test: would a brand-new agent reading this file alone, with no
chat history, understand *why* this code is shaped this way? If yes,
the comment is doing its job.

### R3 — One file, one concept

One concept per file: one struct + its methods + its tests, or one
trait + its derives. When a file accumulates a second unrelated
concept, split.

### R4 — No drive-by refactors, no half-finished implementations

- A bug fix doesn't need surrounding cleanup. A one-shot change
  doesn't need a helper. Three similar lines is better than a
  premature abstraction.
- If you cannot complete a stage, mark it `[!]` in the active session
  doc and halt. Do not commit a partial implementation with a TODO.

### R5 — Tests live with the code

Adding logic in the same commit means adding the test. The runtime
state machine has unit tests per transition; integration tests use
in-memory SQLite plus the `MockRunner` harness in
`codeless-runtime::mock_runner`. PTY / runner tests use a fake
`claude`-style binary on an explicit `PATH` — never the developer's
host install.

## Workflow rules

### Build + verify

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

All three must be green before a commit lands. `clippy -D warnings`
is non-negotiable. MSRV is 1.78 (set on the workspace) — anything
newer must compile-error here, not pass silently.

### Commits + pushes

When a JOB-LOOP loop is running, commit and push **via mani**, never
raw git, and per the rules in [`../DOCS/JOB-LOOP.md`](../DOCS/JOB-LOOP.md):

```sh
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <title>'
./bin/mani --config mani.yaml run push --projects codeless
```

(Both commands run from the workspace root, not from inside this
repo.) Never `--force`, never `--no-verify`. If a hook fails, fix the
cause; do not bypass it. The active session doc must be updated in
the same commit as the code change.

### Branches

Phase 1 work lives on `feat/bootstrap-cargo-workspace`. Later phases
pick a fresh `feat/<phase-slug>` branch.

## Pointers

- Per-repo durable memory: [`CODELESS.md`](./CODELESS.md)
- Workspace agent rules (authoritative): [`../CLAUDE.md`](../CLAUDE.md)
- Project scope: [`../DOCS/SCOPE.md`](../DOCS/SCOPE.md)
- Loop spec: [`../DOCS/JOB-LOOP.md`](../DOCS/JOB-LOOP.md)
- Active session: [`../DOCS/sessions/`](../DOCS/sessions/)
