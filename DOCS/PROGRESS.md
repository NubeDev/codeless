# PROGRESS — where codeless is, what's in flight, what's next

> Snapshot taken 2026-05-13 (late session). Use this when you sit
> down at the keyboard and need to know "what shipped, what's stuck
> where, what should I do next." Supersedes session notes scattered
> under `DOCS/sessions/`.

## What's on master, working

The inner `codeless/` repo at HEAD (`b30663f` `agent_chat: route
footer AI panel to host CLI runners`) carries a real, end-to-end MVP.

### Runtime + UI core
- Single-tenant SQLite source of truth (`jobs`, `stages`, `tasks`,
  `events`, `reviews`).
- `RpcServer` trait + `InProcessRpc` implementation.
- Background job-driver loop with concurrency cap, lease reaper, and
  per-runner factory selection.
- Browser UI on Vite (`http://127.0.0.1:1420`) with the full job
  surface: dashboard, JobPage with three-section sidebar (SPEC /
  STAGES / RUN), per-stage drilldown, header [edit spec] +
  [re-run ▾] dropdown.
- HTTP+SSE transport via `codeless-server`. Loopback unauthenticated;
  non-loopback forces `--require-token`.
- Wire types generated via specta into a TS snapshot
  (`crates/codeless-rpc/tests/wire-rpc.ts.snap`) — drift becomes a
  test failure. Generator is `cargo run -p codeless-rpc --example
  wire_ts`; output mirrored into
  `ui/codeless-ui/src/lib/rpc/generated/wire.ts`.

### Runners
- `MockRunner` — scripted, no process spawn. Drives the entire event
  shape without external dependencies. Now only registered when no
  real runner is enabled (see "mock-strip" below).
- `ClaudeRunnerAdapter` — wraps the host `claude` CLI through the
  vendored `ai-runner` crate. Per-job `model` / `permission_mode` /
  `effort` overrides on the wire and on `Job` rows.
- `AnthropicRunnerAdapter` — direct REST against the Anthropic API.
- `CopilotRunnerAdapter` — GitHub Copilot CLI wrapper.
- `TemplateRunner` — orchestrates multi-stage jobs by spinning up a
  per-stage Claude (or per-stage Mock when `--enable-claude` is off)
  and stitching their events into one job.

### Iterate loop (job-as-directory)
- `.codeless/jobs/<name>/` layout with `template.yaml`, `SCOPE.md`,
  `WORKFLOW.md`, plus any other `.md`. Spec pane in the UI lets the
  user edit any of them inline.
- `list_job_files` / `read_job_file` / `write_job_file` /
  `delete_job_file` RPCs.
- Per-stage docs (each stage's `StageSpec` carries its own ordered
  docs list scoped to that stage); the prompt builder folds per-stage
  docs in after the global SCOPE/WORKFLOW.
- Re-run dialog with optional feedback note.
- Handover panel with the structured editor + `write_handover` RPC.

### Stage rollups (landed previous session)
- `StageRecorder` observer subscribes to the bus and persists `Stage`
  + `Task` rows.
- `list_stages` RPC returns `StageRollup` (stage row + cost rollup +
  task count).
- StageTree renders duration + cost per stage. Header shows totals.
- Click a stage → `StageDetail` right-pane view (placeholders for
  the wishlist items: claude session id, commits, tool ribbon, final
  message — see "Next steps" below).

### Tools (Phase 2-5)
- `codeless-tools` crate — shared policy helpers, ported from moxxy
  with attribution in `NOTICE`.
- `http.request` tool with full policy enforcement.
- `browser.*` tool surface backed by a Playwright sidecar.
- `codeless-mcp` — stdio MCP server exposing the tool surface.

### Demo path
- `codeless demo bootstrap` seeds a repo + a mock job (CLI only).
- `codeless demo bootstrap --target-self` registers the inner repo
  as a Claude-runnable target.
- `DOCS/START-SERVER-UI.md` walks the two-terminal-one-browser flow.

## Recently landed on top of master in this session

These have been merged or committed in the workspace:

### Workspace cleanup
- `35414f9 chore: drop legacy job dirs from earlier sessions` —
  removed `kvstore-111916`, `kvstore-112226`, `spec-demo`,
  `stage-rollup-live` from `.codeless/jobs/` so the file tree only
  shows current jobs.
- A pair of revert commits (`0e8cc65` / `e079c9e`) backed out a
  smoke-test job (`bacnet`) that landed during session work; `7453186`
  / `b0937af` did the same for `demo-job`.

### `agent_chat` (user-side, before this session's work)
- New trait method on `RpcServer` for spawning a CLI runner against
  a free-form prompt with a caller-minted `session_id`. Wired through
  to `HttpRpcClient` + the UI side. The runtime impl lives in
  `crates/codeless-runtime/src/rpc.rs:966+`.

## What's in flight on the working tree (uncommitted)

A large cross-layer change is sitting in the working tree, not yet
committed. **Read this entire section before sitting down to commit.**

### 1. `JobStatus::Draft` state + `start_job` RPC

Submit lands jobs in `Draft` by default; the user edits the spec,
then clicks `[run]` to promote `Draft → Queued`. CLI submits keep
the legacy "submit + run" semantics via `start_immediately: true`.

Touchpoints:
- `crates/codeless-types/src/job.rs` — `JobStatus::Draft` variant.
- `crates/codeless-runtime/src/state_machine.rs` — `Draft → Queued`
  + `Draft → Stopped` legal transitions.
- `crates/codeless-runtime/src/store.rs` — encode/decode arms.
- `crates/codeless-rpc/src/methods.rs` — `SubmitJobArgs.start_immediately`,
  new `StartJobArgs`.
- `crates/codeless-rpc/src/server.rs` — `start_job` trait method.
- `crates/codeless-runtime/src/rpc.rs` — `submit_job` lands jobs in
  `Draft` unless `start_immediately`; `start_job` impl emits the
  long-defined-but-never-emitted `Event::JobPromoted`.
- `crates/codeless-server/src/routes.rs` — `/rpc/start_job` route.
- `crates/codeless-client/src/http_client.rs` — HTTP impl.
- `crates/codeless-cli/src/{demo,job,run}.rs` — pass
  `start_immediately: true` to preserve CLI semantics.
- ~22 test files updated to add the new field; one removed test
  pinned obsolete behaviour.

### 2. Submit-time directory scaffolding

`submit_job` parses `template_yaml` if present and, when it conforms
to `JobTemplate`, scaffolds `.codeless/jobs/<name>/template.yaml`,
`SCOPE.md`, and `WORKFLOW.md` *before* the Job row lands. Single
commit `scaffold job: <name>`. The user never has to "promote a
prompt to a template" — every UI submit lands a full spec.

Touchpoints:
- `crates/codeless-runtime/src/rpc.rs` — `seed_job_directory` helper
  + `SCOPE_PRESET` / `WORKFLOW_PRESET` constants. Called from
  `submit_job` only when the YAML parses.
- `crates/codeless-runtime/tests/job_dir_workflow.rs` — replaced two
  obsolete tests (`list_job_files_reports_none_layout_until_first_save`,
  `write_job_file_creates_directory_layout`) with new ones that
  assert the scaffold contract. Removed the flat-layout migration
  test (no longer reachable through normal flow; migration code
  itself stays for legacy DBs).

### 3. Stop publishing `mock` when a real runner is enabled

A `--enable-claude` server now omits `mock` from the published runner
list AND from the factory's `match` arms. `runner: "mock"` from a
stale UI returns `None` from the factory and the driver fails the
job loudly. The boot log's `runners=…` line was also lying;
fixed.

Touchpoints:
- `crates/codeless-cli/src/serve.rs` — three places gated on
  `real_runner_enabled`.

### 4. SubmitJobDialog rebuild

The submit form now collects **name only** (a slug), auto-derives
the branch as `codeless/<slug>`, builds a minimal `template.yaml`,
and sends `start_immediately: false` by default. The "prompt"
textarea is gone. Yellow warning + greyed dropdown when only `mock`
is enabled. Branch input refuses the repo's `default_branch` (and
`main` / `master` defence-in-depth) with an inline red hint and a
disabled submit button.

`PromoteToTemplate.tsx` is **deleted** — never reachable now.
`SpecPane`'s prompt-only branch renders an explainer for legacy
prompt-only jobs (CLI-submitted) telling the user to submit a fresh
one.

Touchpoints:
- `ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx` — full rebuild.
- `ui/codeless-ui/src/modules/jobs/spec/SpecPane.tsx` — drop
  PromoteToTemplate import + render branch.
- `ui/codeless-ui/src/modules/jobs/PromoteToTemplate.tsx` — DELETED.

### 5. Three-section JobPage sidebar (SPEC / STAGES / RUN)

Replaces the flat 6-tab strip. Each section has a header label + hint.
Stages section is the spine — clicking a stage replaces the right
pane with `StageDetail`. Header gains `[edit spec]` shortcut and
`[re-run ▾]` dropdown (re-run from scratch live; re-run with feedback
+ from-stage are greyed with explicit "not wired yet" subtext).

Touchpoints:
- `ui/codeless-ui/src/modules/jobs/JobPage.tsx` — full layout rewrite.
- `ui/codeless-ui/src/modules/jobs/StageTree.tsx` — `selectedStageId` /
  `onSelectStage` props (additive, optional).
- `ui/codeless-ui/src/modules/jobs/StageDetail.tsx` — NEW. Right-pane
  detail with placeholder cards for the wishlist items.

### 6. SpecPane redesign

Single vertical scroll. Sections in agent-read order: template.yaml
→ SCOPE.md → WORKFLOW.md → other docs. CodeMirror editing in place
(YAML + markdown modes). Per-section save / discard / dirty marker.
Per-stage docs picker (checkbox list). Global docs-order picker
(reorderable checklist). YAML mutated surgically by
`mutateTemplate.ts` to preserve formatting / comments.

Touchpoints:
- `ui/codeless-ui/src/modules/jobs/spec/SpecPane.tsx` — NEW shell.
- `ui/codeless-ui/src/modules/jobs/spec/TemplateSection.tsx` — NEW.
- `ui/codeless-ui/src/modules/jobs/spec/MarkdownSection.tsx` — NEW.
- `ui/codeless-ui/src/modules/jobs/spec/InlineEditor.tsx` — NEW.
- `ui/codeless-ui/src/modules/jobs/spec/parseTemplate.ts` — NEW
  best-effort summary parser.
- `ui/codeless-ui/src/modules/jobs/spec/mutateTemplate.ts` — NEW
  surgical YAML mutators (`setGlobalDocs`, `setStageDocs`).
- `ui/codeless-ui/src/modules/jobs/JobFilesPane.tsx` — DELETED
  (1305 lines replaced by ~1100 lines split across 4 focused files).

### 7. `useJob` refetchable + JobPage subscribes to lifecycle events

`useJob` now returns `{ ...QueryState<Job>, refetch }`. `JobPage`
subscribes to `job-promoted | job-started | job-completed |
job-failed | job-stopped` SSE events for the focused job and calls
`refetchJob()` on each. `start_job` / `stop_job` also call refetch
on RPC success so the badge updates the moment the click returns.
This fixes "click run, click run again, conflict 409" — the badge
flips to `queued` immediately so the second click is impossible.

`SpecPane` got an `afterSave` helper that re-fetches both
`list_job_files` AND the job row, so editing the YAML and saving
makes the template summary re-render with the new content.

Touchpoints:
- `ui/codeless-ui/src/lib/rpc/hooks.ts` — `useJob` extended.
- `ui/codeless-ui/src/modules/jobs/JobPage.tsx` — subscribe + refetch.
- `ui/codeless-ui/src/modules/jobs/spec/SpecPane.tsx` — afterSave.

### 8. JobsDashboard: fix "new jobs invisible until refresh"

When a `job-queued` event arrives for a job not in the dashboard's
initial `useJobs` snapshot, the handler now calls `get_job` and
seeds the overlay so the row appears immediately. `overlayRef`
mirrors state into a ref so the SSE callback doesn't resubscribe
on every overlay change.

Touchpoints:
- `ui/codeless-ui/src/modules/jobs/JobsDashboard.tsx`.

### 9. Browser shell: drop silent mock fallback

Vite-port auto-fallback removed. `MockRpcClient` only when `?mock=1`.
When the server is unreachable, the browser shows a "cannot reach
codeless server" screen with the URL it tried + a retry button.

`RunMockJobButton` and `NewJobDialog` deleted. Top-level "new job"
button removed; the per-repo `SubmitJobDialog` is the only submit
path.

Touchpoints:
- `ui/codeless-ui/src/shells/browser/main.tsx` — rebuild.
- `ui/codeless-ui/src/modules/jobs/JobsDashboard.tsx` — drop top-level
  + per-repo mock button. CTA copy fixed (the original `repos add`
  command had wrong flag syntax).
- `ui/codeless-ui/src/modules/jobs/RunMockJobButton.tsx` — DELETED.
- `ui/codeless-ui/src/modules/jobs/NewJobDialog.tsx` — DELETED.
- `ui/codeless-ui/src/modules/jobs/JobPage.tsx`,
  `ui/codeless-ui/src/modules/jobs/FilesChanged.tsx` — drop two
  stale "mock runner" strings in error fallbacks.

### 10. Two stale UI strings

`JobPage.tsx` and `FilesChanged.tsx` had error-fallback strings
mentioning "mock runner" that were misleading post-strip. Updated.

## Verified

- `cargo build --workspace` ✓
- `cargo test --workspace` ✓ (76 suites pass)
- `tsc --noEmit` (UI) ✓
- `pnpm build` (UI) ✓
- HTTP smoke: submit → Draft, start_job → Queued, second start_job
  → 409 Conflict. submit with `start_immediately: true` → Queued
  immediately. New job from the dialog appears on the dashboard
  without F5. Editing the YAML in the SPEC pane and saving
  re-renders the summary with the new content.

## Sharp edges right now

### The re-run flow doesn't visibly work

`re-run from scratch` calls `rerun_job` and navigates to the new
job's page. But there's no progress indicator, no SSE-driven
status update, and the dropdown gives no feedback that anything
happened. **This is the next session's primary task — see "Next
steps" below.**

### Wrapper-format YAML at submit time

`codeless job submit <file>.yaml` uses a wrapper YAML format
(`repo: ..., runner: ..., stages: [{name: ...}]`) that doesn't
parse as `JobTemplate`. My submit-time scaffold path detects this
and falls through unscaffolded — backwards-compat preserved, but
the CLI submit path no longer gets the spec scaffold. The right
fix is for the CLI to translate its wrapper format into
`JobTemplate` shape before calling `submit_job`. Lower priority
since the UI is the primary submit surface now.

### `useJob` is fetch-once but used in two places per JobPage

`JobPage` and `SpecPane` each mount their own `useJob(jobId)`. Two
`get_job` calls per page open. Acceptable today; correct fix is a
shared cache (TanStack Query). Each mounts its own SSE subscription
too — same shape.

### Branch-conflict if the user picks `default_branch`

The dialog now refuses, but a stuck job can still happen if you
edit branch via the CLI to something that already has a worktree.
Diagnostic: stuck job will sit `Queued` with `worktree_path: None`
and `started_at: None` indefinitely. Recovery: `stop_job` + start
fresh. The driver should surface allocation failures as
`Event::JobFailed` rather than silently leaving the job queued —
follow-up.

### `agent_chat` is real-time-only, no DB persistence

The user-added `agent_chat` RPC spawns a CLI runner against a
caller-minted session id and emits events. Sessions don't persist
across server restarts. Not in scope here; flagged so future
sessions know.

## Next steps

Priority is driven by the constraint that matters most:
[SCOPE.md "Constraint 2"](./SCOPE.md#why-this-scope-the-key-constraints)
— *the coder loop must run unsupervised for hours.* Today the
substrate (worktrees, SQLite, fresh-session model, handover.md
contract, stage/task/review schema, caps) is in place, but the
mechanics that turn that substrate into "go to bed, wake to commits"
are partial. The four autonomy items (A1–A4) close that gap and
take precedence over UX polish. UX items (U1–U2) follow because
they are what the user *sees* of the autonomy work — re-run with
no feedback is invisible to the operator.

Ordering rationale per item is in its own paragraph; if you disagree
with the order, change it here before starting work so the rationale
travels with the change.

The tracks:

| Track | Concern | First items |
|---|---|---|
| **R — Real codebases** | "does it fit a project I actually have?" | R0 (workspace_mode), R1–R4 (polyglot/monorepo, history hygiene, dev loop) |
| **A — Autonomy** | "can it run unattended for 8 hours, and does an interruption inside a stage feel like Claude Code instead of starting over?" | A0 (intra-stage continuation), A1 (cross-stage handover), A2–A5 (feedback, retry, budget, planner) |
| **U — UX** | "can I drive it pleasantly?" | U1–U2 (RunPane, StageDetail wishlist) |

R0 jumps to the absolute top because every other item is moot if a
developer can't actually use codeless against their own repo without
foreign-checkout friction. **A0 sits second** because every other
autonomy item is moot if a mid-stage interruption throws away $5 of
context every time the developer pauses to think — A0 is what makes
the loop feel like Claude Code instead of an alien tool, and the
old "fresh session per session" framing was over-broad. Autonomy
beyond A0 (handover, retry, budget, planner) is moot if you can't
trust the output. UX is moot if you can't use the loop at all. The
order below reflects that.

### R0 — `workspace_mode: in_repo | worktree` (S, blocking real dogfooding)

> Real-codebase claim: *the agent works on **your** repo, on a
> branch you can see, with the dev tools and IDE you already use.*
> Today every job runs in a `/tmp/codeless-worktrees/...` foreign
> checkout — fine for the multi-job overnight use case (which is
> what the original design targeted), wrong for single-developer
> dogfooding (which is how the product actually gets used day to
> day). This is the single biggest "why doesn't this match a normal
> dev workflow" friction.

What changes:

- `Job.workspace_mode: WorkspaceMode { InRepo, Worktree }` — new
  field, **default `InRepo`**. Surfaces on `SubmitJobArgs` so the UI
  / CLI can pick. The submit dialog gets a radio with a sensible
  default (in_repo for personal repos, worktree if explicitly
  flagged or multiple jobs already running).
- `InRepo` mode: the worktree manager skips `git worktree add`.
  Instead it `git checkout -b codeless/<slug>` on the user's existing
  clone (at `Repo.local_path`) and runs the agent there. Edits land
  in the user's real working files. Commits land on the job branch
  in the user's `.git`. `git branch -a`, `git log`, the IDE, and any
  dev server pointed at the repo path all see the change with no
  rebuild-from-tmp-dance.
- `Worktree` mode: today's behaviour — `git worktree add` under
  `.codeless/worktrees/<job-id>` (or `/tmp/codeless-worktrees/...`
  per existing config). Stays opt-in because the overnight / fleet
  use case genuinely needs it.
- **Concurrency rule.** At most one `InRepo` job per repo. Submitting
  a second one returns `RpcError::Conflict("repo X is already in use
  by job Y in in_repo mode; stop it or submit as worktree")`. The
  global / per-repo concurrency caps still apply to `Worktree`
  jobs.
- The job branch survives `InRepo` job termination — it lives in
  the user's repo for them to merge or delete. The system never
  auto-deletes a branch that exists in the user's working clone.

Touchpoints:
- `crates/codeless-types/src/job.rs` — `WorkspaceMode` enum + field.
- `crates/codeless-rpc/src/methods.rs` — wire field on
  `SubmitJobArgs` and `Job`.
- `crates/codeless-runtime/src/worktree.rs` (or wherever worktree
  alloc lives) — branch on mode; in-repo path just does
  `git checkout -b`, no worktree add.
- `crates/codeless-runtime/src/state_machine.rs` — `(repo_id,
  InRepo)` exclusivity check at submit / promote time.
- `crates/codeless-runtime/src/store.rs` — migration to add the
  column with default `'in_repo'` for any backfill (default for
  *new* jobs is also `InRepo`).
- `crates/codeless-server/src/routes.rs` — already covered via
  `submit_job` shape; no new route.
- `crates/codeless-cli/src/job.rs` — `--workspace-mode in_repo |
  worktree` flag (default in_repo).
- `ui/codeless-ui/src/modules/jobs/SubmitJobDialog.tsx` — radio,
  inline help, conflict messaging.
- Tests: state-machine exclusivity (two in_repo submits → second is
  rejected); worktree-alloc skip in in_repo mode; in_repo job's
  branch is left intact on stop.

Out of scope here (later R-track items):
- Per-stage `cwd` / `verify_cmd` for monorepos with multiple build
  systems → R1.
- Path-scoped agent edits per stage → R2.
- Squash-on-merge / sidecar provenance / commit-author rewriting →
  R3.
- Live-reload of the dev server from a job's branch → R4.

Why this is the new floor:

- The original design (worktree-only) is correct for the
  overnight-fleet use case but wrong for the single-developer
  dogfooding loop. Today the dogfooding loop is broken: agent
  edits land in `/tmp`, the user can't test live, and the only way
  to "use" the change is to merge it (which is the *post-merge*
  loop, not the *pre-merge* dev loop).
- Without R0, no amount of A-track work fixes the "doesn't match a
  normal dev workflow" complaint, because the complaint isn't about
  the agent — it's about where the agent works.
- Cost is tiny (a day, maybe two). The schema change is additive.
  Worktree mode stays around verbatim.

### R1 — Per-stage `cwd` + `verify_cmd` (M, monorepo support)

> Real-codebase claim: *a repo with `backend/`, `ui/`, `mobile/`,
> `infra/` works as a single codeless repo.* Today every stage runs
> verify at repo root with one command (whatever the runner
> defaults to). For polyglot or monorepo work that's wrong — a
> `ui/` stage should run `pnpm test` in `ui/`, not `cargo test` at
> root.

Design:

- `StageSpec` gains `cwd: Option<String>` (relative to repo root)
  and `verify_cmd: Option<String>` (single shell line, executed
  in `cwd`). Both default to "repo root" / runner default for
  backward compat.
- The driver passes `cwd` to the runner adapter so the agent's own
  tool calls land in the right place too.
- Per-stage `verify_passed` / `verify_failed` events carry the
  command that ran and its exit code, so the UI can show the
  actual failure without re-running anything.

This is what makes codeless usable on real projects (multi-build
monorepos, the codeless repo itself once UI work needs `pnpm`
verification, etc.). Lower-priority than R0 only because R0 unlocks
*any* dogfooding; R1 unlocks *real-project* dogfooding.

### R2 — Path-scoped edits per stage (S-M)

> Real-codebase claim: *a stage labelled "UI: render session id"
> should not be able to scribble in `crates/`.* Today nothing stops
> the agent from touching anything in the workspace.

Design:

- `StageSpec.scope: Option<Vec<String>>` — glob patterns the stage
  is allowed to edit. Default `None` = no restriction (back-compat).
- The runtime wraps the runner's file-write tool with a guard that
  rejects writes outside `scope`. Tool rejection becomes a
  `tool-call` event with `result: rejected_out_of_scope`.
- Stage SCOPE.md authors can `scope: ["ui/**"]` to keep mechanical
  stages from sprawling.

Lower-priority than R0/R1 because today's three-stage
hand-authored jobs are small enough that scope drift is a review
problem, not a runtime one. Becomes urgent once A5 (planner)
generates stages — auto-generated stages with no scope will sprawl
within hours.

### R3 — Git history hygiene (M, the trust-load-bearer)

> Real-codebase claim: *merging agent work into master does not
> degrade the project's git history.* Today an agent job branch is
> a series of per-stage commits authored by the agent, with
> branch names like `codeless/job-01KRGSA94G9FMZMX7BCPZ06V5G`. Merge
> any of those `--no-ff` and you carry the ULID branch name and
> stage-N commit messages into master forever. Squash-merge and you
> lose the staged review history. Either way, the project's `git
> log` is now polluted with codeless-specific artefacts.

Design (pick one or land both as a user choice):

- **Promote-on-merge:** a `codeless job promote <id>` (and UI
  equivalent) that takes the job branch's commits, presents the
  user with a "this is what's going onto your branch" editor (one
  commit message per stage, or one squashed message), and
  cherry-picks them onto the target branch as the **user**, not the
  agent. The job branch is then archived / discarded. The
  project's history is what the user would have written, with the
  agent's iteration buried in a discarded branch.
- **Sidecar provenance:** every job branch's commits also write
  to `refs/codeless/jobs/<id>` (not `refs/heads/...`), invisible
  to `git branch -a` and `git log --all` unless asked. The branch
  for review purposes still exists, but it doesn't colonise the
  user's normal git surface area.

R3 is *the* trust gate. Without it, a developer who tries codeless
on a serious project finds their `git log` going to pieces and
walks away. With it, the project's history stays the developer's
history and the agent's work is just one more PR-shaped artefact.

### R4 — Live-reload the dev server from a job's branch (S-M)

> Real-codebase claim: *I can run my dev server against the
> agent's work without rebuilding from `/tmp`.* In `in_repo` mode
> (R0) this is trivial — the user's existing dev server already
> sees the agent's edits. In `worktree` mode it's still a gap;
> codeless could add a "rebuild from this job's branch" action
> that runs `cargo run` / `pnpm dev` against the worktree's path.

Lower priority than R0–R3 because R0 makes the most common case
work for free.

### A-1 — `stop` actually stops the running thing (S-M, the integrity fix)

> User claim: *"the stop button must halt whatever the agent is
> doing right now."* Today it doesn't, because there are two
> separate runner spawn paths and the stop control only addresses
> one of them.

The two paths:

- **Job runner.** Spawned by `drive_job` for stages from
  `template.yaml`. Controlled by `start_job` / `stop_job` /
  `pause_job` / `resume_job`. The cap-watcher subscribes to the
  bus and fires the runner's cancellation token on `JobStopped`
  / `JobPaused` / `JobFailed` events (commit `ec1eb63`), so this
  path's cancel is reachable.
- **Chat runner (`agent_chat`).** Spawned every time the user
  clicks `send ▶`. Lives in
  [crates/codeless-runtime/src/rpc.rs:1150](../codeless/crates/codeless-runtime/src/rpc.rs#L1150).
  The `CancellationToken` is created fresh inside the spawned
  task and **never escapes the closure**. No external code can
  fire it. There is no `cancel_chat_task` RPC.

The observable symptom: on a `completed` job whose worktree
still exists, the user clicks `send ▶`, the chat goes
`thinking…`, and there is **no way to stop the agent**. The job
status row says `COMPLETED` (so the action-row's stop button is
hidden — correctly, since the job has nothing to stop), and the
send button becomes `thinking…` with no escape.

The fix is three runtime pieces + one UI change.

**Runtime — three new pieces:**

1. **Chat-cancel registry on the runtime.** `chat_cancels: Arc<Mutex<HashMap<TaskId, CancellationToken>>>`. The `agent_chat`
   spawn inserts the token; a drop-guard removes it on
   completion. Reachable from any RPC handler.
2. **`cancel_chat_task(task_id)` RPC.** Looks up the token in
   the registry and fires it. Idempotent — a missing entry
   (task already finished) is `Ok(())`, not an error.
3. **`stop_active(job_id)` umbrella RPC.** The single RPC the
   UI calls when the user clicks the unified stop button. It
   does both, whichever applies:
   - If the job's status is `Running` / `AwaitingReview` / `Queued`,
     call `stop_job` (writes `Stopped`, cap-watcher cancels the
     job runner).
   - If any in-flight chat task has this `job_id` in scope, fire
     each of those tokens too.
   - Returns a result naming which paths actually fired so the
     UI can report "stopped the chat turn" vs "stopped the job"
     vs both.

**UI — one unified stop button:**

The current action-row `stop ■` is keyed on `job.status === "running"`
and the chat's `send ▶ / cancel` is keyed on the chat's local
`busy` flag. Both become one **stop** button visible whenever
*anything* is running for this job (job runner OR chat turn).
Click → `stop_active`. The send button's transformation into
stop-while-streaming stays, and now actually halts the agent.

**Why "stop" as a single word:**

The user's mental model is "I want this to stop." They do not
care which runner spawn this is — they just want it to stop.
The internal distinction between job stop, job pause, and chat
cancel is real but should not leak into the button label. The
word `cancel` is also intentionally avoided because it suggests
queuing-cancellation, which is something else (`stop_job` on a
queued job).

**Execution plan (5 commits, step-per-commit, verify each):**

1. Runtime: chat-cancel registry + `cancel_chat_task` RPC +
   tests. The registry is an `Arc<Mutex<HashMap<TaskId,
   CancellationToken>>>` mounted on the runtime; agent_chat
   inserts on spawn and removes on completion via a drop-guard.
2. Runtime: `stop_active` umbrella RPC + tests. Reads job
   status, decides which combination of stop_job +
   cancel_chat_task to call, returns a result describing what
   fired.
3. UI: action-row stop button calls `stop_active` instead of
   `stop_job` so it works on chat-turns over a `completed` job.
4. UI: send button becomes stop-while-streaming, also calls
   `stop_active` with the in-flight chat task_id.
5. UI: liveness indicator independent of `job.status` — driven
   by job-running OR chat-busy, so the "something is running"
   signal matches reality.

**Why this lands ahead of A0 and the rest of the autonomy track:**

Every other feature on this roadmap assumes the user has working
control over the agent. "Pause / resume / stop" is the floor.
If clicking stop doesn't stop, no amount of fancier autonomy
work pays off — the user gives up on the tool. A-1 is the
integrity fix that makes the rest of the autonomy track honest.

**Out of scope for A-1:**

- A "pause chat" semantic. Chat turns are one-shot and
  conversational; you can't `--continue` a half-finished
  question, you just ask again. Stop is enough.
- Re-creating the chat-cancel token across server restarts.
  In-flight chat turns die with the daemon; the registry is
  in-memory.
- Multi-tenant scoping. Single trust boundary per SCOPE.md R5.

### A0 — Intra-stage session continuation (M, the load-bearing fix)

> Autonomy claim: *"pause / ask / resume / raise-the-cap inside a
> stage continues the same runner conversation, exactly like
> Claude Code's `--continue`."* Today every mid-stage interruption
> (cost-cap, wall-clock-cap, user stop, daemon restart) spawns a
> fresh agent that has to re-derive the codebase, re-form the
> plan, re-discover what it just learned. That's why a $5 cap
> bump after a $7 stage costs another $7 instead of $0.50 —
> nothing carries. This is the single biggest "doesn't match
> Claude Code" friction, and the previously-stated hard rule
> ("never carry a session token") was the wrong rule.

The corrected rule is in [SCOPE.md hard rule #1](./SCOPE.md#hard-rules-for-the-coding-runner):
**the stage is the session boundary, not every runner
invocation.** Within a stage the runner session is continuous;
across a stage boundary the session always resets. Stages bound
context; sessions do not have to.

What changes:

1. **Pause vs stop as a job/stage state distinction.** New
   terminal-vs-pause distinction in `JobStatus` and `Stage.status`.
   Cost-cap and wall-clock-cap mid-stage transition to
   `paused (cost-cap)` / `paused (wall-clock-cap)`, not `failed`
   / `stopped`. The worktree, the branch, and the captured
   `Stage.session_id` survive. The job is *resumable*, not
   *re-runnable*.
2. **`resume_job` RPC.** Takes `{ job_id, additional_cost_cap_cents?,
   additional_wall_clock_cap_ms? }`. Increments the existing caps
   (or leaves them) and re-fires the stage's runner with
   `--continue <session_id>` — same conversation, same in-context
   files, same half-formed plan. The agent wakes up where it left
   off.
3. **Cap raise on a paused job.** Either via `resume_job`'s
   optional cap fields, or a separate `update_job_caps` RPC.
   Either way the existing job row is mutated; no clone, no fresh
   worktree.
4. **"Ask a question" inside a stage.** A user message during a
   paused stage gets folded into the next prompt before the
   `--continue` resumes. Reuses the same plumbing as A2's
   `add_job_note`, but scoped to the *current* stage's session
   rather than the next stage's prompt.
5. **The captured `Stage.session_id` is load-bearing, not
   observability.** The doc-comment on
   `crates/codeless-types/src/stage.rs` (cherry-picked into
   `feat/stage-session-id`) currently says *"Persisted for
   observability only — see SCOPE.md hard rule #1: codeless never
   reuses this to resume a runner."* That's wrong under the new
   rule. Update the comment when A0 lands so the next contributor
   doesn't think the field is decorative.

Touchpoints:
- `crates/codeless-types/src/job.rs` — `JobStatus::Paused(PauseReason)`.
- `crates/codeless-types/src/stage.rs` — `StageStatus::Paused`,
  update the docstring on `session_id` to match the new rule.
- `crates/codeless-runtime/src/state_machine.rs` — new transitions
  (`running → paused (cap)`, `paused → running` on resume).
- `crates/codeless-runtime/src/scheduler.rs` (or wherever the cap
  check fires) — pause-not-fail on cap trip mid-stage.
- `crates/codeless-runtime/src/template_runner.rs` /
  `claude_runner.rs` — when the stage row has a non-null
  `session_id`, the next task's CliCfg sets
  `claude_session_id: Some(...)` so the wrapper passes
  `--continue`. Provider-specific equivalents for Anthropic /
  OpenAI runners come later (REST resume is shaped differently;
  out of scope for the first A0 slice — CLI-wrapper Claude is
  enough to ship the UX).
- `crates/codeless-rpc/src/{server,methods}.rs` — `resume_job`
  RPC and args.
- `crates/codeless-server/src/routes.rs` — `/rpc/resume_job` route.
- `crates/codeless-client/src/http_client.rs` — HTTP impl.
- `crates/codeless-cli/src/job.rs` — `codeless job resume <id> [--cost-cap-bump
  ¢] [--wall-clock-bump ms]`.
- UI: paused-state badge on JobsDashboard and the RunPane, a
  `[resume]` action with an inline cap-bump field when the pause
  reason is cap-tripped.

Out of scope here (A1's job):
- Cross-stage handover synthesis — that's still A1, just with
  cleaner framing: handover is the artefact *between* stages, not
  the artefact for every interruption.
- REST-runner (`AnthropicRunner`, `OpenAIRunner`) resume —
  different mechanism (re-send the conversation; no provider
  session token). First A0 slice covers the CLI-wrapper path,
  which is the dominant use case and where the friction was
  observed.

Why this is ahead of A1:

- A1's old framing said "handover at every session boundary." But
  most "session boundaries" in current operation are actually
  pauses (cap trips, user stops, restarts) — and a handover-style
  reset across a pause throws away ~$5 of context for ~$0 of safety
  benefit, because the *next* invocation is the same stage doing
  the same work. The handover artefact has a real job — it's the
  cross-stage onboarding doc — but it's the wrong tool for the
  intra-stage problem.
- Without A0, the developer-loop complaint stands: every "I need
  to stop and think for a second" costs another $5 to restart.
  Nobody dogfoods a tool that wastes their context every time
  they look away.
- A0 is genuinely simpler than A1 — no synthesiser, no event
  rollup, no diff-summarisation. Just a status enum extension, a
  resume RPC, and `--continue` passthrough. A week of work,
  schema-additive.

### A1 — Cross-stage handover (M-L, after A0)

> Autonomy claim: *"at a stage boundary the next stage's fresh
> session reads a handover that names what landed, what halted,
> and what was learned, then picks up correctly."* This is the
> **cross-stage** onboarding artefact. The intra-stage continuation
> problem belongs to A0 — they are different problems and the old
> framing of A1 conflated them.

What exists today:

- `write_handover` RPC and the structured editor in `SpecPane` —
  the *operator* can write handover.
- `.codeless/jobs/<name>/` job-as-directory layout, including
  `runs/<name>/handover.md` per [SCOPE.md "Directory and repo
  layout"](./SCOPE.md#directory-and-repo-layout).
- Per-stage docs folded into the prompt by the prompt builder.

What's missing — must land together:

1. **Agent-authored handover at stage completion.** When a stage
   reaches a terminal state that hands control off to the *next
   stage* (`passed`, or `failed` with no retry, or
   `awaiting-review` once the review resolves and the next stage
   opens), the runtime synthesises `handover.md` from: the stage's
   events, the commit diff on the branch, the final assistant
   message (`StageDetail` wishlist item #4), and a structured
   "what landed / what's open / what to watch out for" block.
   Deterministic summarisation over `events` + `git log`, not a Rig
   helper; Rig can polish later. **Cap-tripped and crashed
   mid-stage do not synthesise a handover** — those are A0's
   pauses, the same stage continues with `--continue`.
2. **Prompt builder reads handover first.** The next session's
   system prompt opens with handover.md (if present), then SCOPE,
   then WORKFLOW, then the stage's per-stage docs. Order matters:
   handover is the only thing that knows what just happened.
3. **Handover lives on the branch.** Push it with the stage commit
   — per [SCOPE.md:331](./SCOPE.md#L331) "push every stage, never
   defer", the remote *is* the handover state. A session that
   re-clones can still pick up.
4. **Verify by example.** `bacnet`-style smoke tests pass already;
   the real verify is a job that fails stage 2, where session 3
   reads the handover and resumes correctly. Add a demo
   (`handover-resume`) under `codeless/.codeless/jobs/` proving this.

Touchpoints (expected):
- `crates/codeless-runtime/src/handover.rs` — NEW. Deterministic
  synthesiser over events + git diff.
- `crates/codeless-runtime/src/job_driver_loop.rs` — call synthesiser
  at session end; pass handover path into prompt builder.
- `crates/codeless-runtime/src/prompt.rs` (or wherever the prompt
  builder lives) — handover-first ordering.
- `crates/codeless-cli/src/git.rs` (or runtime equivalent) — handover
  goes into the stage commit, not a separate one.

### A2 — Re-run with feedback folded in (S-M, partially scoped)

> Autonomy claim: *"re-run with feedback folded in."* Today the
> dropdown shows it greyed with "needs add_job_note RPC". This is
> the smallest closed-loop control the operator has over a running
> job — without it the only feedback channel is "stop and start
> over."

Wiring (unchanged from the previous snapshot's plan):

- New `add_job_note` Rust RPC + wire types + server route + HTTP impl.
- Notes-folding in `job_driver_loop.rs`'s prompt builder (TODO at
  line ~205). Note becomes a clearly-marked block in the next
  session's prompt, *after* the handover (handover is what happened;
  note is what the operator wants different).
- UI: textarea in the re-run dropdown → `add_job_note` then
  `rerun_job`. Re-run navigates to the new job's RunPane.
- MCP parity: `codeless.job.rerun` carries an optional `note` field
  per [AGENT-CONTROL-PLANE.md](./AGENT-CONTROL-PLANE.md) Slice 2.

Sequencing: ship the RPC + folding *before* the UI wires it. CLI
gets `codeless job rerun <id> --note "..."` on the same change.

### A3 — Verify-fail policy: agent decides retry vs. escalate (M)

> Autonomy claim: *"verify-fails-loop: agent decides retry vs.
> escalate."* Today a failed stage halts the job. For long runs
> this is too brittle — one flaky test stops the night. The fix is
> declarative policy on the stage, not agent-side decisioning
> (agent-side decisioning is unbounded and burns budget).

Design:

- `StageSpec` gains `on_verify_fail`:
  - `retry_with_feedback` (max N, default 2) — synthesise a
    feedback note from the verify output and fire a fresh session
    on the same stage. Counts against the job's wall-clock + cost
    caps.
  - `escalate_to_review` — materialise a `Review` row, halt the
    stage in `awaiting-review`, emit `review-requested`. This is
    where the operator's morning queue comes from.
  - `fail_stage` (today's behaviour, kept as the default for
    backward compat).
- Verify output flows into the synthesised feedback note via the
  same handover-builder pipeline (A1). Don't write a second
  summariser.
- Per-job cap is the floor: retries inside a stage cannot exceed
  the job's cost/wall-clock budget. Cap-tripped wins over retry.

Touchpoints:
- `crates/codeless-types/src/stage.rs` — `OnVerifyFail` enum.
- `crates/codeless-rpc/src/methods.rs` — wire type.
- `crates/codeless-runtime/src/state_machine.rs` — new transitions
  (`verify-failed → running` for retry; `verify-failed →
  awaiting-review` for escalate).
- `crates/codeless-runtime/src/job_driver_loop.rs` — policy
  evaluation.
- UI: `StageDetail` shows retry count + policy; spec pane lets the
  user pick policy per stage.

Order: ship A1 first. The retry path *is* the handover path with
a synthesised note — it cannot work cleanly until handover does.

### A4 — Loop-level aggregate cost ceiling (S, currently post-MVP)

> SCOPE.md:379 calls this out as the highest-priority budget feature
> after per-job caps. Twenty jobs × per-job cap is the surprise
> bill on waking. Pull it forward to land alongside A3 — both are
> cheap, both are *required* for unattended use.

Design:

- Two-tier ceiling per [SCOPE.md "Loop-level aggregate ceilings"](./SCOPE.md#cost-visible-and-cappable-from-day-one):
  `Continue` / `Escalate` / `Halt`.
- Evaluated by the runtime *before firing each session* — at the
  scheduler tick, not inside a runner. Cheaper than per-token
  enforcement and matches how per-job caps already work.
- Config in `~/.config/codeless/limits.toml` (next to auth.toml).
  Defaults: warn at $20/day cumulative, halt at $50/day. User
  edits or disables freely.
- `Escalate` materialises a job-level `Review` row; the loop pauses
  until the operator clears it.

Touchpoints:
- `crates/codeless-types/src/limits.rs` — NEW.
- `crates/codeless-runtime/src/scheduler.rs` — pre-fire check.
- `crates/codeless-server/src/routes.rs` — `get_limits`,
  `set_limits` RPCs.
- UI: dashboard header shows cumulative-vs-ceiling bar.

### U1 — Chat-as-control center for a job (M-L, depends on A0 for full power; ships in degraded form earlier)

The design supersedes the original "RunPane overhaul" framing.
**The chat window becomes the control center** — every event the
agent emits and every action the user takes lives in one
scrolling, live conversation, with run / pause / stop / resume
inline in the composer. The right pane is ambient signal
(status, cost, runtime, drill-down tabs), not the primary
surface. Same UX shape as Claude Code / Cursor / Copilot, because
that shape is the one that survives "I look away for a minute,
come back, pick up."

Full load-bearing decisions, layout, phasing, and SSE
reliability fixes are in [`JOBS-UX.md`](./JOBS-UX.md). Summary
of the five phases there:

1. **SSE reliability** (S, 1-2 days) — `id: <cursor>` on every
   SSE event, `Last-Event-ID` honouring, heartbeats, connection-
   state badge, live elapsed-time tick, `task-completed`
   subscription for live cost. Ships standalone; biggest
   immediate win for any existing job. Fixes the "messages stop
   coming and I refresh" complaint that was the loudest signal
   in the 2026-05-13 dogfood session.
2. **ConversationPane** (M-L, 3-5 days) — unify timeline + chat
   + stage transitions into one streaming pane.
3. **Composer with state-driven actions** (M, 2-3 days) — move
   run/pause/stop into the composer; state machine drives the
   primary button label.
4. **Right-pane refactor** (S-M, 2 days) — status + tabs as
   ambient signal; spec editing moves out of the per-job tabs.
5. **A0 integration** — flips pause/resume from degraded to real
   when the A0 runtime work lands. No UI rewrite needed.

The original framing's claim was correct ("UX over a broken loop
is animation") and the same logic now gates *parts* of U1 on A0:
phases 1-4 ship before A0 with the composer's pause / resume
buttons in a degraded "queue for next session" mode; phase 5
upgrades them in place once A0 is done.

Old "Approach" notes for the original RunPane framing have been
folded into [`JOBS-UX.md`](./JOBS-UX.md) — see "Migration from
the current implementation" there for how today's `RunPane` /
`JobChat` / `StageDetail` components map onto the new layout.

### U2 — Wishlist items for `StageDetail` (M, parallelisable with A1)

Placeholder cards exist (see `StageDetail.tsx`); wire the data.
These compose with A1 — the handover synthesiser reads commits,
final messages, and tool calls anyway, so the data plumbing is
shared. Under the U1 redesign these cards live inside the
right-pane Stages tab; the card *shape* survives, the parent
surface changes.

1. **Claude session ID** per stage. **Done** on
   `feat/stage-session-id` (`NubeDev/codeless`): `Stage.session_id`
   wire field, `Event::StageSessionCaptured`, `StageRecorder`
   capture, `Captured` card in `StageDetail`. Migration `0003`
   ships the SQLite column. Pending merge to master.
2. **Per-stage commits.** `git log <branch>` joined to stage
   timestamps.
3. **Tool-call ribbon.** Roll up `Event::ToolCall` per stage at
   query time (no schema change needed).
4. **Final assistant message excerpt.** Buffered in the claude
   adapter today; needs an event or persistence.

### A5 — Planner: goal → stages (L, deliberately last)

> Autonomy claim: *"planner generates stages from a goal."* The
> aspiration in [SCOPE.md:611](./SCOPE.md#L611) is correct but
> deliberately scheduled last:
>
> - Per the same SCOPE entry, "defer until ~10 real jobs exist to
>   train against — a bad plan poisons the whole job."
> - Per [SCOPE.md "Helper role" hard rule #1](./SCOPE.md#helper-role--rig-optional-never-gates-a-job),
>   the loop must work end-to-end with zero helpers configured.
>   Planner is a helper, not a gate.
> - Without A1 (handover) and A3 (verify-fail policy), an
>   auto-generated plan failing mid-way leaves the operator with no
>   recovery surface anyway.

Land it after A1–A4 have produced a corpus of real `runs/*/log.md`
that a Rig planner can RAG against. First version: take user goal +
the repo's `CODELESS.md`, emit a `template.yaml` draft, *user
reviews before submit*. No auto-submit, ever.

### Smaller / housekeeping items (any time)

These are real but they don't move the autonomy needle. Pick them
up when the autonomy track is between phases.

- **Re-run from a specific stage (L).** Needs the Job/Run schema
  split (JOB-WORKFLOW.md Phase B). Bigger than a single session.
  Park until there's a real driver — most likely once the morning
  review queue exists and the user wants "redo stage 3 with this
  note" as a verb.
- **CLI submit translates wrapper YAML → JobTemplate.** So
  `codeless job submit` jobs also get the disk scaffold. Small
  focused change in `crates/codeless-cli/src/job.rs`. Lower
  priority since the UI is the primary submit surface now.
- **Driver surfaces worktree-allocation failures (S).** Today a
  job whose `default_branch` is taken sits silently in `Queued`.
  Should emit `Event::JobFailed` with a clear reason. Becomes more
  urgent once unattended runs are real — a stuck-queued job at 3am
  is invisible.
- **`agent_chat` persistence.** Real-time-only today, no DB
  persistence. Not in scope for the autonomy track; flag if a user
  starts relying on it.

## Documentation that's now stale (housekeeping)

- `DOCS/JOB-DIR-KICKOFF.md` — references the destroyed session and a
  rebuild plan that's now done. Delete or mark "historical".
- `DOCS/JOB-DIR.md` "What this rebuilds" section at the bottom is
  obsolete.
- `DEMO-UI.md` `ux-12`–`ux-15` entries describe an iterate-loop UI
  that's been substantially rebuilt twice since.
- This snapshot supersedes any older `DOCS/sessions/` notes.

## TL;DR for the next session

**Open work right now is A-1.** A1–A0–rest only matter if the
basic "stop" control works. Today the stop button only addresses
the *job* runner; chat turns (`agent_chat`) spawn a separate
runner whose `CancellationToken` is unreachable, so on a
`completed` job the user can send a chat message and there is
no way to halt the agent. See A-1 in **Next steps** above for
the full plan — registry of in-flight chat tokens, `stop_active`
umbrella RPC, unified UI stop button. Five focused commits,
step-per-commit verification.

After A-1 lands the remaining order is unchanged:

1. R-track ("does it fit a project I actually have?") with **R0
   (`workspace_mode`)** at the top.
2. Within A-track, autonomy (A1–A4) precedes the RUN page UX
   overhaul (U1), because UX over a still-broken loop is
   animation. A0 (intra-stage session continuation via
   `--continue`) is already landed on the `feat/jobs-ux-composer`
   branch alongside the pause/resume work; ready to merge once
   the stacked branches are reviewed.
3. The user's working tree may still contain uncommitted
   cross-layer work (Draft state + scaffolding + UI rebuild).
   Decide whether to commit it as-is or split into the 10
   numbered chunks above. Don't push yet without explicit user
   OK — two pairs of revert commits in recent history show the
   user wants control over what lands.
4. **Start on A-1.** Five commits, schema-additive, in-memory
   registry only (no migration). Then re-pick from R0 onward.

Earlier ordering (kept for context — supersede with the above):
4. Then **A0 — intra-stage session continuation.** This is the
   "pause / ask / resume / raise-the-cap" fix that makes codeless
   feel like Claude Code instead of an alien tool. Without it,
   every mid-stage interruption costs another $5 to re-derive the
   codebase. Schema-additive (`Paused` job/stage status, a
   `resume_job` RPC, `--continue <session_id>` passthrough to the
   Claude wrapper), CLI-Claude-only first slice. The captured
   `Stage.session_id` on `feat/stage-session-id` becomes
   load-bearing here, not observability — update its docstring as
   part of A0 to reflect that.
5. **A1 — cross-stage handover.** With A0 in place, A1 has a
   crisper job: the *next stage* needs to onboard from the
   *previous stage*'s output. Cap-pauses and crashes are no
   longer A1's problem; they're handled by A0. Verify with a real
   demo job under `codeless/.codeless/jobs/handover-resume/`
   where stage 1 commits, stage 2 opens fresh, reads the handover,
   does its job correctly.
6. A2 (`add_job_note` + folding), A3 (verify-fail policy), A4
   (loop-level cost ceiling) round out the autonomy track.
7. U1 (RUN page UX) and U2 (StageDetail wishlist) follow once
   R0 + A0 + A1–A2 are in. U2 can run in parallel with A1 since
   the handover synthesiser reads the same data the wishlist
   cards render.
8. R1–R4 (per-stage cwd/verify, path scoping, git-history
   hygiene, dev-server live-reload) sit behind R0 and pick up
   when a real polyglot project is the next test target. **R3
   (git history hygiene) is the trust gate** — without it,
   developers won't merge agent work onto serious projects, so
   land it before the first overnight run on anything that isn't a
   throwaway repo.
