# CHAT-EDITS-JOB-SPEC — letting the chat agent author the job spec

> Design doc. Not yet implemented. Status: proposed, awaiting decision
> on §4.

## Why

Today the job spec — `template.yaml`, `SCOPE.md`, `WORKFLOW.md`,
per-stage docs — is authored by hand in the Spec pane. The chat
panel already reads the spec (see [the prompt-fold work in
`codeless-runtime/src/rpc.rs`](../codeless/crates/codeless-runtime/src/rpc.rs)
under `load_chat_job_spec`) but cannot write it.

The user-visible goal is:

> "Create an empty job, then in chat say *'add a stage to run cargo
> bench and capture flamegraphs, write a SCOPE that prioritises p99
> over throughput'* — and the agent edits the spec into shape before
> the user clicks **run**."

This collapses the spec-authoring loop. The user becomes a *director*:
describe what success looks like, let the agent shape stages, docs,
verify commands, model picks. The user reviews the diff and runs.

## What "the spec" actually is

For one job named `<name>`, in repo `<repo>` at `local_path`:

```
<local_path>/.codeless/jobs/<name>/
├── template.yaml        — name, goal, stages[].title, per-stage docs:
├── SCOPE.md             — load-bearing job description (folded into every stage prompt)
├── WORKFLOW.md          — how the agent should drive the work
├── CHAT.md              — append-only transcript (written by chat, not user-edited)
└── *.md (optional)      — per-stage docs the template references in stages[].docs
```

The SQLite source-of-truth duplication:

- `template.yaml` is also mirrored into the `jobs.template_yaml` DB
  column. The runtime parses this at submit + run time; the on-disk
  file is what `git log` records but the DB is what the driver reads.
- `SCOPE.md` / `WORKFLOW.md` / other `.md` files are read **from disk
  on demand** by `TemplateRunner` (per the docs list in `template.yaml`
  and the job-level `docs:`). No DB cache.
- `CHAT.md` is written by the chat handler after every turn — the
  transcript is the file.

This duplication is the **first design constraint**: any tool that
edits `template.yaml` must round-trip through the existing
`update_job_template` RPC, or the next driver tick reads a stale
DB row. Direct disk edits to `template.yaml` are silently lost.

## Current state

| Surface | Reads spec | Writes spec |
|---|---|---|
| Spec pane (UI) | yes — `list_job_files` / `read_job_file` | yes — `write_job_file` / `update_job_template` / `delete_job_file` |
| Job run (driver) | yes — `TemplateRunner` folds spec into every stage prompt | no — runner edits source code, not the spec |
| Chat panel (footer per-job chat) | **yes** (the change we just landed) | **no** — chat agent is read-only on the spec |

Claude does have ambient filesystem write access in the chat — the
runner is invoked with `--allow-all-tools` — but a direct
`Edit('.codeless/jobs/<name>/template.yaml', …)` from chat would
leave `jobs.template_yaml` in SQLite stale.

## Three options

### Option A — chat agent edits via filesystem, codeless re-syncs

The chat agent uses the tools it already has (`Edit`, `Write`). A
file watcher on `<local_path>/.codeless/jobs/` notices changes and
calls `update_job_template` / refreshes the in-memory cache.

- **Pros:** zero new agent-side surface; the agent uses the same
  primitives it already knows.
- **Cons:** filesystem watcher in `codeless-adapters-host`,
  cross-platform footgun. Validation is post-hoc — the agent can
  write malformed YAML and only learn at the next driver tick. No
  audit of *intent* (every write looks like a generic edit).

### Option B — give the chat agent codeless-specific tools (recommended)

Define a small, named tool surface the chat agent can call. Wire
them through MCP (already plumbed in
[ai-runner's claude runner](../ai-runner/src/runners/claude.rs#L121-L138))
or as in-prompt tool definitions:

| Tool | Effect |
|---|---|
| `job_set_template(yaml: str)` | `update_job_template` RPC. Validates YAML, errors with parser message. |
| `job_write_file(filename: str, content: str)` | `write_job_file` RPC. Sanitises filename, refuses `template.yaml` (use `job_set_template`). |
| `job_delete_file(filename: str)` | `delete_job_file` RPC. |
| `job_read_file(filename: str)` | `read_job_file` RPC — only useful if the file isn't already in the preamble. |
| `job_list_files()` | `list_job_files` RPC — surface what exists. |

The agent is told in its preamble: "to edit the spec, call these
tools, not Edit/Write directly." When the agent does call Edit on a
spec file, the preamble tells it the call will be ignored.

- **Pros:** clean validation path (same path the Spec pane uses);
  audit trail (each tool call is a discrete event); the chat
  agent's *intent* is legible in the transcript; SQLite stays
  consistent.
- **Cons:** new tool surface to maintain; the agent has to learn it
  (the preamble does the teaching, but a new agent might still try
  raw `Edit` first).

### Option C — chat agent edits via filesystem, SCOPE.md / WORKFLOW.md only; `template.yaml` stays user-only

Restrict the agent to the markdown docs. The user keeps full control
over `template.yaml`. Validation is moot because Markdown has no
structure to break.

- **Pros:** simplest possible; the high-risk file (template) stays
  human-authored.
- **Cons:** half a feature — the user's quoted ask ("add a stage to
  run cargo bench") needs `template.yaml` edits.

## §4 — recommendation

**Option B**, narrowed to four tools in v1:

1. `job_set_template(yaml)`
2. `job_write_file(filename, content)`
3. `job_delete_file(filename)`
4. `job_list_files()`

`job_read_file` is omitted in v1 — the preamble already includes
`template.yaml`, `SCOPE.md`, `WORKFLOW.md`. If a per-stage doc is
needed, the agent can grep for the filename and ask the user, or we
extend the preamble to include all `.md` files under the job dir
(cheap; capped by `MAX_CHAT_SPEC_BYTES`).

Tool *implementation* can route either through MCP (the cleanest;
the `claude-wrapper` already accepts `--mcp-config`) or as
codeless-internal tool definitions injected via the runner's
`CliCfg.allowed_tools`. The choice is an implementation detail.
Recommended: MCP, so the same tool surface is reusable for Codex /
Copilot later.

## Knowledge the chat agent needs

The preamble needs a primer block — separate from the spec-fold we
already added. Place it under a `# Job-spec authoring` heading
before `# Context`, only when the chat is per-job (skip for the
footer panel). Contents:

```
# Job-spec authoring

This chat is attached to job `<name>` (id `<id>`, status `<status>`)
under repo `<local_path>`. You can shape its spec by calling these
tools:

- `job_set_template({yaml})` — replace the full template. Required
  fields: `name` (must equal `<name>` — renames are rejected),
  `goal` (string), `stages` (non-empty list, each with `title:` and
  optional `docs:`). Optional job-level `docs:` list lifts files
  attached to *every* stage's prompt.
- `job_write_file({filename, content})` — create or overwrite a
  supporting doc (e.g. `SCOPE.md`, `WORKFLOW.md`, `per-stage-foo.md`).
  Filename is sanitised: lowercase letters, digits, `-`, `.md`
  extension auto-applied. Refuses `template.yaml`.
- `job_delete_file({filename})` — remove a supporting doc.
- `job_list_files()` — list what currently exists.

Conventions:

- `SCOPE.md` is the load-bearing scope, folded into every stage.
- `WORKFLOW.md` is how the agent should drive (per-stage protocol,
  end-of-stage gate, drift rules).
- Per-stage docs are referenced from `stages[i].docs:` in the
  template and folded only into that stage's prompt.

Do NOT edit `template.yaml`, `SCOPE.md`, `WORKFLOW.md`, or any
spec file via raw `Edit`/`Write` — those edits will be silently
ignored. Use the tools above so SQLite stays consistent.

`CHAT.md` is owned by the runtime; do not write it.
```

Roughly 600 bytes. Adds to but does not displace the existing job
spec fold.

## Guardrails

1. **Name immutability.** The wire already rejects renames in
   `update_job_template`; the preamble tells the agent so it doesn't
   try.
2. **No starting the job from chat.** The chat agent has no
   `job_start` tool. Promotion `Draft → Queued` stays a user gesture
   (the **run** button). Reason: the user reviews the final spec
   before paying for a run. Adding `job_start` later is fine but
   not v1.
3. **No cap edits from chat.** Similarly, `cost_cap_cents` /
   `wall_clock_cap_ms` stay user-only. Until we ship the
   `update_job_caps` RPC mentioned in `SubmitJobDialog`, there is
   nothing to wire to anyway.
4. **No cross-job edits.** Tools are scoped to the chat's
   `session_id` (== job_id). The handler refuses if the caller
   tries to write to another job's directory.
5. **Audit.** Every spec-edit tool call lands a `tool-call` event
   on the bus, so the job report already in §Summary tab records
   *what* the chat did. (Today's report counts them; future work:
   include a "spec changes" rollup grouping by tool name and
   filename.)
6. **Draft-only?** Optional stricter mode: refuse spec edits when
   `status != Draft`. Prevents the agent from rewriting a running
   job's scope mid-execution. Recommended off by default — a paused
   or failed job sometimes wants a spec tweak before resume — but
   gated behind a per-server flag.

## UX in the UI

When a chat-driven spec edit lands:

1. The runtime emits a `job-file-updated` or
   `job-template-updated` event (same shape as the Spec pane's own
   writes).
2. The Spec pane's `useReviews`-style hook listens for those events
   on the current job and refetches `list_job_files` /
   `read_job_file` / `get_job` on hit. Already partly there for
   user-side edits; verify it covers chat-driven edits too.
3. The chat side renders the tool call inline in the transcript so
   the user can see "agent wrote SCOPE.md" without leaving the
   chat.

No new panel. The existing Spec pane is the diff view.

## Rollout

1. **Land the preamble primer behind a feature flag.** Same fold
   path as `load_chat_job_spec`, gated on `CODELESS_CHAT_JOB_EDITS=1`
   until the tools work. Lets a developer turn it on per-server.
2. **Implement the four tools as an MCP server** in
   `codeless-adapters-host` exposed at a loopback `127.0.0.1:<port>`
   the chat runner is pointed at via `CliCfg.mcp_url` /
   `CliCfg.mcp_token`. Reuse the existing `MCP_URL` plumbing.
3. **Authenticate the MCP server** with a per-chat-turn token
   minted by the runtime and embedded in the runner's env, so a
   different process on the same host can't impersonate the chat.
4. **Wire the Spec-pane refetch** for `job-template-updated` and
   `job-file-updated`. Likely a 1-line change to whatever hook
   already drives Spec on user edits.
5. **Add the "spec changes" rollup** to the job report (§Summary)
   so the user can see "chat made 3 SCOPE.md writes, 2 template
   updates" at a glance.
6. **Per-server flag** for "draft-only" mode (§Guardrails 6).

Each step ships independently. After step 2, the user can already
say "add a stage for X" in chat and have it land in `template.yaml`.

## Open questions

1. **MCP vs in-runner tool definitions.** MCP is cleaner but adds a
   tiny loopback server. In-runner tools (passed via
   `CliCfg.allowed_tools` + tool-definition prompt block) avoid the
   server but couple the tool surface to the Claude wrapper's
   tool-spec format. Recommend MCP; revisit if the loopback adds
   friction on Windows.
2. **Other runners (Codex / Copilot).** MCP is supported by Claude
   but Copilot's CLI and Codex's CLI don't speak it. v1 limits
   spec-edit chat to Claude; the dropdown can hide other runners
   when the chat is on a per-job panel. v2: a translation layer that
   maps "I want to add a stage" → tool call works for any runner
   that supports tool-use.
3. **Markdown vs structured edits.** Today `job_write_file` takes a
   full `content` string. For long docs the agent re-emits the
   whole file every edit, which is wasteful and prone to drift. v2
   could add `job_patch_file(filename, edits[])` taking a list of
   `{old, new}` pairs (the same shape `Edit` already uses) — much
   cheaper for incremental tweaks.
4. **History.** The Spec pane could show "last edited by chat" vs
   "last edited by user" if we tag commits. Today every write is a
   git commit (`update job-file: ap/SCOPE.md`); add a `chat:` /
   `spec:` prefix to disambiguate.
5. **Concurrent edits.** If the user is in the Spec pane editing
   SCOPE.md while the chat agent is also editing it, the last
   writer wins (SQLite-wise) and the loser's edits land as conflict
   markers (git-wise, on the next commit). v1 ignores this — it's
   single-user, single-session in MVP. v2 if it bites: optimistic
   concurrency on `updated_at`.
6. **Should the chat be able to *read* arbitrary repo files via
   tool?** Today the agent uses ambient `Read` because the cwd is
   the repo. That's fine. Out of scope for this doc.

## Pointers

- Existing spec-fold: [`codeless-runtime/src/rpc.rs`](../codeless/crates/codeless-runtime/src/rpc.rs) — `load_chat_job_spec`, `build_chat_prompt`
- Existing job-file RPCs: [`codeless-rpc/src/methods.rs`](../codeless/crates/codeless-rpc/src/methods.rs) — `WriteJobFileArgs`, `UpdateJobTemplateArgs`, `DeleteJobFileArgs`, `ReadJobFileArgs`
- MCP config plumbing: [`ai-runner/src/runners/claude.rs`](../ai-runner/src/runners/claude.rs) — `mcp_tmp_path`
- Chat dispatch: [`codeless-adapters-host/src/ai_chat.rs`](../codeless/crates/codeless-adapters-host/src/ai_chat.rs)
- The user's original ask (this doc's motivation): see `DOCS/HACKLINE-DEV.md` history; the chat panel's blind-spot was identified during the hackline goals 4–5 run.
