# Event publish-site audit

Inventory of every `EventBus::publish(...)` call in production source
under `crates/codeless-runtime/`. Built for milestone 2 of
`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md` ("Publish-site
audit"): the M5 server-side `EventFilter::Repo(RepoId)` filter is only
implementable at fan-out if every envelope can be resolved back to a
`RepoId`. The reviewer's "6 RPCs, 1 day" estimate holds iff that
resolution is direct; anywhere we have to invent a fallback is real
scope.

## Bus signature

```rust
EventBus::publish(
    job_id: Option<JobId>,
    stage_id: Option<StageId>,
    task_id: Option<TaskId>,
    event: Event,
    now: UnixMillis,
) -> sqlx::Result<EventCursor>
```

`repo_id` is **not** a column on the `events` table and **not** a
parameter on `publish`. Anything M5 can route on must come from either
(a) a field inside the `Event` payload, or (b) a join the fan-out
filter runs on the `job_id` it already has (`jobs.repo_id`). Sites
where neither path resolves are the audit's red rows — they need a
schema change before M5 can land.

## Event-kind taxonomy

Pulled from `crates/codeless-types/src/event.rs`.

### Library-scope (carry `repo_id` in payload, no owning job)

| Event variant | Notes |
|---|---|
| `RepoAdded { repo_id }` | Library list refresh. |
| `RepoRemoved { repo_id }` | Library list refresh. |
| `RepoUpdated { repo_id }` | Emitted by `attach_workspace` / `detach_workspace`. |
| `WorkspaceUnhealthy { repo_id, fs_root, reason }` | Liveness sweep. |
| `WorkspaceRecovered { repo_id, fs_root }` | Liveness sweep. |

These are the only variants the wire already tags by repo. M5's
`EventFilter::Library` channel exists exactly for this set.

### Repo-scope, payload carries `repo_id`

| Event variant | Notes |
|---|---|
| `JobQueued { job_id, repo_id }` | The one job-lifecycle variant with repo on the wire. |

### Repo-scope, payload does **not** carry `repo_id` (must resolve via `jobs.repo_id`)

Job lifecycle:

- `JobPromoted { job_id }`
- `JobStarted { job_id }`
- `JobCompleted { job_id }`
- `JobStopped { job_id, reason }`
- `JobFailed { job_id }`
- `JobPaused { job_id, reason }`
- `JobResumed { job_id, previous_reason, actor }`
- `JobReset { job_id, previous_status }`
- `JobPolicyChanged { job_id, policy_name }`
- `JobTemplateUpdated { job_id }`
- `JobFileUpdated { job_id, filename }`
- `ChatMessageAppended { job_id, message }`
- `ChatBindingCreated { transport, channel_id, thread_id, job_id }`

Stage / task / todo / verify / scope-patch-proposed:

- `StageStarted`, `StageCompleted`, `StageTrioGateWaiting`,
  `StageSessionCaptured`, `SessionArchivedThenResumed`
- `VerifyStarted`, `VerifyPassed`, `VerifyFailed`,
  `VerifyStepStarted`, `VerifyStepPassed`, `VerifyStepFailed`,
  `VerifyStepSkipped`
- `TaskEnqueued`, `TaskStarted`, `TaskCompleted`
- `ToolCall`, `ToolApprovalRequested`, `AiToken`, `AiMessageComplete`
- `TodoAdded`, `TodoUpdated`, `TodoCompleted`
- `ScopePatchProposed`
- `StageAutoBypassed`
- `ReviewPreCheck`, `ReviewVerdict`

For all of these the publish site passes `Some(job_id)` as the
envelope's `job_id` field, so the fan-out filter can resolve repo
with one indexed lookup against `jobs.repo_id`. **No payload change
required.**

### Red rows — neither payload `repo_id` nor envelope `job_id` resolves a real repo

These are the call sites that block M5 from being "one server-side
WHERE clause". They split into three families.

**Family A — stage-scoped publish with `job_id = None`.** The
envelope carries only `stage_id`, so fan-out has to join through
`stages.job_id → jobs.repo_id` to land on a repo.

- `Event::ReviewApproved { review_id }` —
  `crates/codeless-runtime/src/rpc/reviews.rs:60`
- `Event::ReviewCommented { review_id, comment }` —
  `crates/codeless-runtime/src/rpc/reviews.rs:90`
- `Event::ReviewStopped { review_id }` —
  `crates/codeless-runtime/src/rpc/reviews.rs:108`
- `Event::ScopePatchApproved { stage_id, … }` —
  `crates/codeless-runtime/src/rpc/scope_patches.rs:116`
  (uses `synthetic_stage_id()` — there is no `stages` row to join,
  the stage id is fabricated for envelope shape)
- `Event::ScopePatchRejected { stage_id, … }` —
  `crates/codeless-runtime/src/rpc/scope_patches.rs:174`
  (synthetic stage id, same caveat)

`approve_scope_patch` / `reject_scope_patch` already know their
`RepoId` (it's an RPC arg). The two synthetic-stage sites are the
only ones in the codebase that fabricate an id specifically to fit
the envelope shape — they want a repo-scoped channel they don't
have.

**Family B — assistant planner / thread events (synthetic
`bus_job_id`).** The envelope `job_id` is set to
`JobId(thread_id.0)`, which is **not** a row in `jobs`. The
`jobs.repo_id` join returns nothing.

- `Event::AssistantThreadTouched { thread_id }` —
  `crates/codeless-runtime/src/rpc/assistant.rs:39`
  (`bus_job_id = JobId(thread_id.0)`)
- `Event::AssistantThreadTouched { thread_id }` —
  `crates/codeless-runtime/src/auto_bypass_failure_card.rs:117`
  (same synthetic id)
- Planner-streamed events (`AiToken`, `ToolCall`,
  `ToolApprovalRequested`, `AiMessageComplete`) emitted by the
  forwarder closure at
  `crates/codeless-runtime/src/rpc/assistant_planner.rs:168`
  (`bus_job_id = JobId(thread_id.0)`)

Assistant threads do not yet have a `repo_id` of their own; they
are library-scope from the UI's point of view. M5 has to either
(a) treat the assistant channel as `Library`, or (b) thread a real
`repo_id` onto `assistant_threads`. The BROWSER-LAUNCHER plan
implicitly assumes (a) — sidebar lives across repos — but the
filter contract needs to spell it out.

**Family C — UI chat session events (envelope `job_id = session_id`).**
`rpc/chat.rs:153` forwards adapter events with the chat session id
in the `job_id` slot:

- `bus.publish(Some(session_id), None, Some(task_id), event, now_ms())`
  at `crates/codeless-runtime/src/rpc/chat.rs:153`

The forwarded variants are `AiToken` / `ToolCall` /
`ToolApprovalRequested` / `AiMessageComplete`. `session_id` is a
free-running id that does not correspond to a `jobs` row, so the
`jobs.repo_id` join fails the same way the assistant planner does.
This surface predates the assistant-thread split; M5 needs to
classify it either as `Library` or attach a real `repo_id` to chat
sessions.

## Call-site inventory

One row per production publish call. Test publishes (`#[cfg(test)]`
modules + `tests/*.rs` integration tests) are explicitly excluded —
they do not feed real subscribers, and the assertion that an event
"carries `repo_id`" is about the production wire only.

Columns: `path:line` · event kind · envelope `(job_id, stage_id,
task_id)` · repo resolution at fan-out time.

Repo-resolution legend:

- **payload** — `Event` payload field already carries `repo_id`.
- **via job** — envelope `job_id` is a real `jobs.id`; resolve with
  `jobs.repo_id`.
- **via stage** — envelope `stage_id` is a real `stages.id`; resolve
  with `stages.job_id → jobs.repo_id`.
- **needs schema** — neither path works; M5 cannot route this site
  without a wire / data-model change.

### Library scope

| Site | Event | Envelope | Resolution |
|---|---|---|---|
| `crates/codeless-runtime/src/rpc/repos.rs:23` | `RepoAdded` | `(None, None, None)` | payload |
| `crates/codeless-runtime/src/rpc/repos.rs:39` | `RepoRemoved` | `(None, None, None)` | payload |
| `crates/codeless-runtime/src/rpc/workspaces.rs:118` | `RepoUpdated` (attach) | `(None, None, None)` | payload |
| `crates/codeless-runtime/src/rpc/workspaces.rs:209` | `RepoUpdated` (detach) | `(None, None, None)` | payload |
| `crates/codeless-runtime/src/workspace_liveness.rs:154` | `WorkspaceUnhealthy` | `(None, None, None)` | payload |
| `crates/codeless-runtime/src/workspace_liveness.rs:164` | `WorkspaceRecovered` | `(None, None, None)` | payload |

### Job lifecycle

| Site | Event | Envelope | Resolution |
|---|---|---|---|
| `crates/codeless-runtime/src/rpc/jobs.rs:133` | `JobQueued` (submit) | `(Some(job), None, None)` | payload + via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:175` | `JobPromoted` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:293` | `JobResumed` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:654` | `JobStopped` (`stop_job`) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:772` | `JobPaused` (`pause_job`) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:836` | `JobPolicyChanged` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:914` | `JobReset` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:969` | `JobQueued` (`rerun_job`) | `(Some(job), None, None)` | payload + via job |
| `crates/codeless-runtime/src/rpc/jobs.rs:1164` | `JobTemplateUpdated` (resync) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/driver.rs:104` | `JobStarted` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/driver.rs:202` | `JobCompleted` \| `JobFailed` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/driver.rs:499` | `JobPaused` \| `JobStopped` (cap-watcher) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/job_driver_loop.rs:567` | `JobQueued` (retry re-publish) | `(Some(job), None, None)` | payload + via job |
| `crates/codeless-runtime/src/job_driver_loop.rs:621` | `JobFailed` (`mark_job_failed`) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/scoped_pause_hook.rs:274` | `JobPaused` (scoped pause point) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/supervisor/tools/actions.rs:321` | `JobStopped` (supervisor stop_run) | `(Some(job), None, None)` | via job |

### Job-file / job-template

| Site | Event | Envelope | Resolution |
|---|---|---|---|
| `crates/codeless-runtime/src/rpc/job_files.rs:141` | `JobFileUpdated` (write) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/job_files.rs:173` | `JobFileUpdated` (delete) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/job_files.rs:249` | `JobTemplateUpdated` | `(Some(job), None, None)` | via job |

### Chat / chat binding

| Site | Event | Envelope | Resolution |
|---|---|---|---|
| `crates/codeless-runtime/src/rpc/job_chat.rs:69` | `ChatMessageAppended` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/rpc/job_chat.rs:156` | `ChatBindingCreated` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/supervisor/tools/mod.rs:309` | `ChatMessageAppended` | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/supervisor/tools/actions.rs:426` | `ChatMessageAppended` (`post_supervisor_row`) | `(Some(job), None, None)` | via job |

### Stage / verify / task / todo / scope-patch-proposed (per-job runners)

| Site | Event | Envelope | Resolution |
|---|---|---|---|
| `crates/codeless-runtime/src/template_runner.rs:2461` (`publish` helper) | `StageStarted` / `StageCompleted` / `StageTrioGateWaiting` / `StageAutoBypassed` / `ReviewPreCheck` / `ReviewVerdict` / others raised by the template runner | `(Some(job), Some(stage), Some(task))` | via job |
| `crates/codeless-runtime/src/verify_runner.rs:236` (`publish` helper) | `VerifyStarted` / `VerifyPassed` / `VerifyFailed` / `VerifyStep*` | `(Some(job), Some(stage), Some(task))` | via job |
| `crates/codeless-runtime/src/trio_emitter.rs:270` (`publish` helper) | `TodoAdded` / `TodoUpdated` / `TodoCompleted` | `(Some(job), Some(stage), Some(task))` | via job |
| `crates/codeless-runtime/src/claude_runner.rs:294` | `AiToken` / `ToolCall` / `ToolApprovalRequested` / `AiMessageComplete` (event forwarder) | `(Some(job), None, Some(task))` | via job |
| `crates/codeless-runtime/src/claude_runner.rs:357` | `StageSessionCaptured` | `(Some(job), Some(stage), Some(task))` | via job |
| `crates/codeless-runtime/src/codex_runner.rs:61` | runner-stream events (`AiToken` etc.) | `(Some(job), None, Some(task))` | via job |
| `crates/codeless-runtime/src/anthropic_runner.rs:63` | runner-stream events | `(Some(job), None, Some(task))` | via job |
| `crates/codeless-runtime/src/copilot_runner.rs:59` | runner-stream events | `(Some(job), None, Some(task))` | via job |
| `crates/codeless-runtime/src/mock_runner.rs:58` | mock-runner events (test fixtures use this in prod-path tests too) | `(Some(job), None, None)` | via job |
| `crates/codeless-runtime/src/scope_patch_emit.rs:136` | `ScopePatchProposed` | `(Some(job), Some(stage), None)` | via job |
| `crates/codeless-runtime/src/session_idle.rs:298` | `SessionArchivedThenResumed` | `(Some(job), Some(stage), None)` | via job |

### Red rows — needs a schema or contract change before M5 can route

| Site | Event | Envelope | Why it does not resolve |
|---|---|---|---|
| `crates/codeless-runtime/src/rpc/reviews.rs:60` | `ReviewApproved` | `(None, Some(stage), None)` | `job_id` omitted; M5 must add a join via `stages.job_id` or land `job_id` on the envelope. |
| `crates/codeless-runtime/src/rpc/reviews.rs:90` | `ReviewCommented` | `(None, Some(stage), None)` | same as above. |
| `crates/codeless-runtime/src/rpc/reviews.rs:108` | `ReviewStopped` | `(None, Some(stage), None)` | same as above. |
| `crates/codeless-runtime/src/rpc/scope_patches.rs:116` | `ScopePatchApproved` | `(None, Some(synthetic_stage), None)` | `stage_id` is fabricated; no row to join. The RPC already has `args.repo_id` — payload should carry it. |
| `crates/codeless-runtime/src/rpc/scope_patches.rs:174` | `ScopePatchRejected` | `(None, Some(synthetic_stage), None)` | same as above. |
| `crates/codeless-runtime/src/rpc/assistant.rs:39` | `AssistantThreadTouched` | `(Some(JobId(thread_id.0)), None, None)` | synthetic `bus_job_id`; no `jobs` row. Treat as `Library` or thread `repo_id` onto assistant threads. |
| `crates/codeless-runtime/src/auto_bypass_failure_card.rs:117` | `AssistantThreadTouched` | `(Some(JobId(thread_id.0)), None, None)` | same as above. |
| `crates/codeless-runtime/src/rpc/assistant_planner.rs:168` | planner-streamed `AiToken` / `ToolCall` / `ToolApprovalRequested` / `AiMessageComplete` | `(Some(JobId(thread_id.0)), None, Some(task))` | same as above. |
| `crates/codeless-runtime/src/rpc/chat.rs:153` | adapter-streamed `AiToken` / `ToolCall` / `ToolApprovalRequested` / `AiMessageComplete` | `(Some(session_id_as_job), None, Some(task))` | `session_id` is not a `jobs.id`; same classify-or-extend choice as the assistant family. |

## Counts

- Library-scope production publish sites: **6**.
- Repo-scope sites that resolve via payload `repo_id`: **3** (the
  three `JobQueued` emit sites).
- Repo-scope sites that resolve via envelope `job_id` → `jobs.repo_id`:
  **30**.
- Red-row sites (cannot resolve as the wire stands today): **11**
  — five stage-only publishes (reviews + scope-patch approve/reject)
  and six assistant / chat sites with a synthetic id in the
  envelope.

## Reading for the M5 plan

The "6 RPCs, 1 day" estimate holds for the green half — every
job-lifecycle, job-files, job-template, chat-on-job, stage, verify,
task, todo, scope-patch-proposed, and runner-stream publish already
carries a real `job_id` on the envelope. A single
`SELECT repo_id FROM jobs WHERE id = ?` (or the existing fan-out
filter joining `events` to `jobs`) is enough to route them.

The red rows are real work and split into two follow-ups:

1. **Reviews + scope-patch approve/reject.** Five sites. The
   minimal fix is to add `Some(job_id)` to the envelope at the
   publish call (the RPC already has it on the row, or the args).
   No wire-format change to `Event`. Cost: hours, not days; same
   change-set as M5 itself.

2. **Assistant + chat sessions.** Six sites that route through a
   synthetic id. These are explicitly *library* surfaces today —
   the assistant rail lives across repos. The cleanest M5 contract
   is to declare assistant / unbound-chat envelopes `Library` and
   let the per-repo subscription ignore them. If a future
   "assistant thread bound to a workspace" feature lands, the
   underlying `assistant_threads` row gets a `repo_id` and these
   sites flip to `EventFilter::Repo` at that point. M5 should land
   the classification rule, not the data-model change.

Net: M5's route-by-`repo_id` filter at fan-out is **one
SELECT-or-join**, not a wire-format expansion. The estimate stands;
the two red-row families above are the only adds.
