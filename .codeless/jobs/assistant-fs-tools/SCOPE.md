# Scope — assistant-fs-tools

The in-app Assistant currently has no filesystem access. Asking it
"draft a job that refactors X" forces the user to paste paths the
assistant cannot verify; the assistant cannot read existing scopes,
cross-check references, or ground its suggestions in the actual
repo state. That is the bad UX this job fixes.

This job is the **read/write filesystem surface** for the Assistant
planner. It builds on the assistant work that already landed (see
[`DOCS/ASSISTANT-SCOPE.md`](../../../../DOCS/ASSISTANT-SCOPE.md)
status block, 2026-05-15) — F2 wired the planner via `agent_chat`,
F3 added action-card dispatch, F1 hooked the footer bar. The
missing piece is filesystem tools in the planner's registry.

## Goal

Three new `Tool` impls registered on the planner's registry for
assistant threads, plus three more behind a per-thread permission
mode, so the assistant can read the workspace by default and write
to it when the user opts in. Permission vocabulary aligns with
Claude Code / Copilot CLI / Codex so the user carries one model
across tools.

## In scope

- `fs.list(path)` — directory listing, workspace-root-relative.
- `fs.read(path)` — file read, capped at a reasonable byte limit.
- `fs.search(query, glob?)` — ripgrep-style content search; pure-Rust
  walker (`ignore` + `grep-searcher`) to satisfy R1.
- `fs.write(path, content)` — create-or-overwrite a file.
- `fs.edit(path, old, new)` — exact-string replace, fails on
  ambiguity.
- A workspace-root path sandbox shared by all six tools — rejects
  absolute paths and any `..` traversal. Same shape as precheck
  rule #3's worktree-resolve layer in
  [`crates/codeless-runtime/src/diff_verify.rs`](../../../crates/codeless-runtime/src/diff_verify.rs).
- `mode` column on `assistant_threads` (default `read-only`),
  migration, plus `assistant.setThreadMode(thread_id, mode)` RPC.
- Write paths gated server-side on the thread's `mode`:
  - `read-only` — write tools return a typed error.
  - `approve-edits` — each `fs.write` / `fs.edit` surfaces as an
    `AssistantActionCard` and runs only after the user confirms via
    the existing `confirm_assistant_action` dispatcher.
  - `bypass` — writes execute immediately.
- **Special case in every mode**: writes under
  `<workspace_root>/.codeless/jobs/<name>/` route through the
  existing `jobs.updateScope` RPC, never through `fs.write`. That
  RPC enforces the paused-job rule; bypassing it would silently
  corrupt running jobs.
- UI: mode dropdown in the `/assistant` context panel (right rail
  per ASSISTANT-SCOPE §1) bound to `assistant.setThreadMode`. The
  displayed mode is the server's value, not a local cache.

## Out of scope

- Filesystem tools for the **runners** (Claude / Copilot / Codex
  CLIs spawned per stage). Those already have their own
  permission flags inside their CLIs; codeless wraps them but
  does not re-implement their permission model. This job is the
  **assistant planner's** tool surface only.
- Multi-tenant per-user permissions. Single-tenant trust boundary
  (R5) holds — the mode is per-thread, not per-user.
- Process spawning. `fs.search` uses a pure-Rust walker; ripgrep is
  not invoked. Spawning `rg` would force the tool to live in
  `codeless-adapters-host` (R1) and add a runtime dependency on a
  binary that may not be installed.
- A separate "plan mode" à la Claude Code. The three modes above
  cover the same surface (read-only is plan mode for our purposes).
- Audit log / undo for `bypass` writes. Bypass is "I trust this
  thread"; `git` is the audit trail.

## Constraints

- **R1** — process spawn lives only in `codeless-adapters-host`.
  The tool implementations live in `codeless-tools` and reach the
  filesystem through `tokio::fs` / `std::fs`; no `Command`.
- **R2** — the UI mode dropdown calls `RpcClient.assistant.setThreadMode`,
  not `fetch` and not Tauri.
- **R3** — one responsive component for the dropdown; no per-shell
  fork.
- **R4** — `mode` lives in SQLite. The dropdown subscribes; it does
  not maintain its own truth.
- **R5** — the bearer token authorises the assistant identically
  to every other client; mode is a per-thread setting, not a
  per-user grant.
- The mode check is **server-side** on tool dispatch. A client
  reporting `bypass` over a thread persisted as `read-only` must
  not be able to write. Same pattern as the assistant `kind` prop
  (ASSISTANT-SCOPE §2): UI hints, server enforces.

## Permission-mode naming — alignment with the field

The three modes match the well-known vocabulary so users do not
have to learn a fourth permission model:

| Codeless mode    | Claude Code                | Copilot CLI         | Codex CLI         |
|------------------|----------------------------|---------------------|-------------------|
| `read-only`      | `default` (no writes) / `plan` | read-only mode  | `read` mode       |
| `approve-edits`  | `acceptEdits`              | confirm-each       | `suggest` (default) |
| `bypass`         | `bypassPermissions`        | yolo / auto-approve | `auto-edit`       |

The user picks one of three; the table is documentation only —
codeless does not expose the upstream names directly.

## Deliverables

- `codeless-tools/src/tools/fs_tools.rs` (or a `fs/` submodule) with
  six `Tool` impls + the path sandbox helper, registered in
  [`codeless-tools/src/tools/mod.rs`](../../../crates/codeless-tools/src/tools/mod.rs).
- `assistant_threads.mode` column + migration in
  `codeless-runtime`.
- `assistant.setThreadMode` on `RpcServer` in `codeless-rpc`, dispatch
  in `codeless-runtime`.
- Server-side mode check at tool dispatch — write tools refuse on
  `read-only`; `approve-edits` writes return a "confirmation
  required" outcome that becomes an action card.
- Mode dropdown in
  [`codeless/ui/codeless-ui/src/modules/assistant/`](../../../ui/codeless-ui/src/modules/assistant/)
  context panel.
- Tests:
  - Sandbox helper unit tests (absolute path reject, `..` reject,
    symlink-out-of-root reject if cheap).
  - Per-tool unit tests in `codeless-tools`.
  - Integration tests in `codeless-runtime` using `MockRunner`:
    one per mode × one per tool covering happy path + reject path.
  - Round-trip test that a confirmed `fs.write` action card writes
    exactly once.
  - Test that an `fs.write` path under `.codeless/jobs/<name>/`
    routes through `jobs.updateScope`, in every mode.

## Open questions (resolve in stage 1)

1. **Symlinks crossing the workspace root.** Resolve before
   sandbox check, or trust the path-prefix check alone? Bias:
   resolve before check; symlinks pointing outside the root must
   be rejected. Cost: one `tokio::fs::canonicalize` per tool call.
2. **`fs.read` size cap.** Bias 5 MiB. Files larger than the cap
   return a "too large, narrow the request" error rather than
   streaming.
3. **`fs.search` result cap.** Bias 200 matches, truncation
   noted in the result. Bigger searches force the planner to
   narrow.
4. **What does the action card show for `fs.write` confirmation?**
   Bias: file path + a unified diff against the current file (or
   "new file" if absent), same component as `jobs.updateScope`'s
   diff card. Re-uses what F3 already built; no new UI primitive.
5. **Should `read-only` mode hide write tools entirely from the
   planner's tool list, or expose them and have them reject?**
   Bias: hide. The planner should not propose actions the user
   has declined to allow; surfacing them is noise. Cost: tool
   registry construction reads the thread row.

Record decisions in this file under "Decisions" before stage 3
begins.

## Decisions

The job goal called out three load-bearing confirmations and the
section above listed five smaller open questions. All eight are
resolved below. Stage 3 onward treats this section as authoritative;
re-opening a decision means amending it here in the same commit as
the code change that motivates the reversal.

### Top-level (from the job goal)

- **D1 — Mode names: `read-only` / `approve-edits` / `bypass`.**
  Confirmed as the bias. The three strings are the wire values
  stored in the DB and accepted by `assistant.setThreadMode`; they
  match Claude Code's `default` / `acceptEdits` / `bypassPermissions`
  closely enough that users carrying that model arrive without
  retraining. The naming table in "Permission-mode naming" above
  documents the cross-tool mapping for the docs surface; the wire
  vocabulary itself stays Codeless-native (we do not expose
  upstream tool names). The constraint is **exactly three modes** —
  no "plan" alias, no `acceptEdits` synonym, no client-side
  remapping. A server-side enum (`AssistantThreadMode`) gates parsing
  so a typo on the wire is a hard reject, not a silent fallback.

- **D2 — Thread-mode storage: a `mode TEXT NOT NULL DEFAULT
  'read-only'` column on `assistant_threads`.** Confirmed.
  Single source of truth, per-thread, server-owned. A separate
  table (`assistant_thread_modes`) was considered and rejected: it
  buys nothing (1:1 with the parent row), forces a join on every
  tool dispatch, and complicates the `setThreadMode` write path.
  Existing rows backfill to `read-only` via the migration's
  `DEFAULT`, which matches the goal of "safe by default". The
  column accepts only the three D1 strings; enforcement is the
  server-side enum, not a `CHECK` constraint (keeps the migration
  reversible and sidesteps SQLite's `CHECK`-rewrite limits if D1
  ever grows a fourth mode).

- **D3 — `.codeless/jobs/<name>/` routes through `jobs.updateScope`
  in every mode, including `bypass`.** Confirmed. The check lives
  at the `fs.write` / `fs.edit` dispatch boundary in
  `codeless-runtime`, not inside the `Tool` impl, so the routing
  decision is made with the thread row, the workspace root, and
  the resolved path all in hand. Detection: after the sandbox
  resolves the target path, if the path lies under
  `<workspace_root>/.codeless/jobs/<segment>/` (any depth), the
  call short-circuits to `jobs.updateScope` with `(job_name =
  <segment>, relative_path = <tail>, content = <new contents>)`
  and the tool returns the `jobs.updateScope` outcome verbatim.
  `bypass` does **not** opt out — the paused-job rule is a runtime
  invariant, not a permission. `fs.edit` against an existing
  job-scoped file reads the current content, applies the
  exact-string replace in memory, and forwards the result to
  `jobs.updateScope`; the edit primitive is preserved without
  letting a second write path exist.

### Open-question resolutions (from the §"Open questions" list)

- **D4 (Q1) — Resolve symlinks before the sandbox check.** The
  sandbox helper canonicalises the requested path (via
  `tokio::fs::canonicalize` for existing paths, or canonicalises
  the deepest existing ancestor and re-joins the tail for create
  paths) and then re-applies the workspace-root prefix check
  against the canonical form. Symlinks pointing outside the root
  are rejected with the same error shape as a literal `..`
  traversal. Cost is one `canonicalize` per tool call; the
  alternative — trusting a prefix check on the raw input —
  leaves an obvious escape via a checked-in symlink and is
  rejected.

- **D5 (Q2) — `fs.read` size cap: 5 MiB.** Files larger than the
  cap return a typed `too_large { path, size, cap }` error with
  the message "file exceeds 5 MiB cap; narrow the request (line
  range or search instead)". No partial / streaming read; the
  planner is meant to operate on diffs, not blobs. The cap is a
  constant in `codeless-tools::fs` so a future override is a
  single-file change.

- **D6 (Q3) — `fs.search` result cap: 200 matches, truncated with
  a flag.** The result shape carries `{ matches: Vec<Match>,
  truncated: bool, total_seen: u32 }`. When `truncated` is true,
  the planner sees the count and is expected to narrow (a tighter
  glob, a more specific query). 200 matches at typical line
  lengths fits inside the planner's context budget with room for
  the conversation; raising the cap forces context-window
  pressure on the assistant.

- **D7 (Q4) — Action-card payload reuses the
  `jobs.updateScope` diff card.** `fs.write` confirmation on an
  `approve-edits` thread surfaces an `AssistantActionCard` whose
  body is `{ path, before: Option<String>, after: String }`. The
  existing diff-renderer component (F3, shared with
  `jobs.updateScope`'s card) draws a unified diff; "new file"
  when `before` is `None`. No new UI primitive. `fs.edit`
  pre-renders the post-replace content into `after` so the card
  shows the literal future state, not the `(old, new)` tuple.

- **D8 (Q5) — `read-only` hides write tools from the registry.**
  The planner's tool list is built per-call from the thread's
  current mode. On `read-only`, `fs.write` / `fs.edit` are not
  registered at all; the assistant never sees them, never
  proposes them, and the user is not presented with an action it
  cannot take. The cost — one extra column read on registry
  construction — is paid once per tool-call boundary and is
  cheaper than an action-card surface that would have to render
  a "rejected" state. Server-side rejection still exists as
  defence-in-depth: a stale client that *does* call `fs.write`
  on a `read-only` thread receives the same typed error as
  before (D1's enum gate), it just never gets a tool definition
  to invoke.

### Implications for later stages

- Stage 3's migration writes `mode TEXT NOT NULL DEFAULT
  'read-only'`. No `CHECK`; the enum guards parsing.
- Stage 4's read-only tools (`fs.list`, `fs.read`, `fs.search`)
  consume D4–D6: canonicalising sandbox, 5 MiB cap, 200-match cap.
- Stage 6's `fs.write` / `fs.edit` consume D3 (job-scope routing)
  and D7 (action-card payload shape). The job-scope routing test
  must cover `bypass` explicitly — that is the regression D3
  exists to prevent.
- Stage 7's UI dropdown shows exactly the three D1 strings and
  binds to the server enum.
