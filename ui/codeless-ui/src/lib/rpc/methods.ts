// RPC method args + results — hand-mirrored from
// `codeless/crates/codeless-rpc/src/methods.rs`. Specta currently
// generates only the wire types in `wire.ts`; method-arg structs are
// not yet in the codegen output. When that lands, replace this file.

import type {
  AgentChatArgs,
  AgentChatResult,
  ApproveScopePatchArgs,
  AutoBypassPolicy,
  CancelChatTaskArgs,
  EditScopePatchArgs,
  EventCursor,
  FsEntry,
  FsGlobHit,
  FsGrepHit,
  FsReadResult,
  FsStatArgs,
  FsStatResult,
  GitAuth,
  Handover,
  Job,
  JobDiffArgs,
  JobDiffResult,
  JobId,
  ListProposedPatchesArgs,
  ListProposedPatchesResult,
  RejectScopePatchArgs,
  Repo,
  RepoId,
  Review,
  RevertScopePatchArgs,
  RevertScopePatchResult,
  ScopePatchActionResult,
  StageId,
  ShellBgEntry,
  ShellBgLogChunk,
  ShellCommandOutput,
  StopActiveArgs,
  StopActiveResult,
  UploadChatAttachmentArgs,
  UploadChatAttachmentResult,
  ShellSessionRunOutput,
  AssistantThread,
  AssistantMessage,
  ListAssistantThreadsArgs,
  ListAssistantThreadsResult,
  CreateAssistantThreadArgs,
  DeleteAssistantThreadArgs,
  UploadAssistantAttachmentArgs,
  UploadAssistantAttachmentResult,
  ListAssistantMessagesArgs,
  ListAssistantMessagesResult,
  AppendAssistantMessageArgs,
  AppendAssistantMessageResult,
  AssistantAction,
  AssistantActionCard,
  AssistantActionStatus,
  ConfirmAssistantActionArgs,
  ConfirmAssistantActionResult,
  CancelAssistantActionArgs,
  CancelAssistantActionResult,
} from "./wire";

export interface AddRepoArgs {
  name: string;
  clone_url: string;
  default_branch: string;
  local_path: string;
  git_auth: GitAuth;
  concurrency_cap: number | null;
  default_runner: string | null;
}

export interface RemoveRepoArgs {
  repo_id: RepoId;
}

export interface ListReposResult {
  repos: Repo[];
}

export interface SubmitJobArgs {
  repo_id: RepoId;
  prompt: string | null;
  template_yaml: string | null;
  runner: string;
  branch: string;
  /** `in-repo` (default) edits the user's local clone; `worktree`
   * creates a separate `git worktree add` checkout. */
  workspace_mode?: "in-repo" | "worktree" | null;
  cost_cap_cents: number;
  wall_clock_cap_ms: number;
  /** Optional per-job model id. Adapters that don't understand it
   * ignore it; `null` means "use the runner's default." */
  model?: string | null;
  /** Per-call permission mode. Today only the Claude adapter honours
   * it; valid values: `default | accept_edits | plan | bypass`. */
  permission_mode?: string | null;
  /** Thinking-budget hint: `low | medium | high`. */
  effort?: string | null;
  /** Persona-derived system prompt composed at submit time. The
   * runtime applies this on top of the server's baseline system
   * prompt for every stage; `null` keeps the server default. */
  system_prompt?: string | null;
  /** Id of the persona the user picked, persisted alongside the
   * already-resolved `system_prompt` so a rerun reproduces the same
   * agent posture even if the persona's body is edited later.
   * `null` means no persona was picked. */
  persona_id?: string | null;
  /** Per-job auto-bypass policy (Surface F). `null` (default) keeps
   * the existing halt-on-failure behaviour. A preset (`quick`,
   * `long-term`, `cheap`, `best-judgement`, `just-code`) or a
   * `custom` free-text comment pre-authorises the runtime to advance
   * past a non-cap stage failure with the policy's guidance threaded
   * into the next stage's prompt. Cap-breach failures always halt. */
  auto_bypass_policy?: AutoBypassPolicy | null;
  /** When `false` (default) the job lands in `Draft` status — the row
   * exists, the user can edit the spec / docs / handover, but the
   * driver does not pick it up. The user calls `start_job` to promote
   * the job to `Queued`. `true` queues the job for immediate run, the
   * legacy / power-user behaviour. */
  start_immediately?: boolean;
}

export interface GetJobArgs {
  job_id: JobId;
}

export interface ListJobsArgs {
  repo_id: RepoId | null;
}

export interface ListJobsResult {
  jobs: Job[];
}

// Stage rollup returned by `list_stages`. Stage row carries the
// canonical persistent state (status, started_at, ended_at, name,
// ordinal); cost_cents and task_count are derived rollups from the
// stage's child task rows.
export interface StageRollup {
  stage: {
    id: string;
    job_id: JobId;
    ordinal: number;
    name: string;
    status: "pending" | "running" | "awaiting-review" | "passed" | "failed";
    verify_cmd: string | null;
    started_at: number | null;
    ended_at: number | null;
    session_id: string | null;
  };
  cost_cents: number;
  task_count: number;
}

export interface ListStagesArgs {
  job_id: JobId;
}

export interface ListStagesResult {
  stages: StageRollup[];
}

export interface JobReportArgs {
  job_id: JobId;
}

export interface JobReportStage {
  ordinal: number;
  attempt: number;
  title: string;
  status: string;
  session_id: string | null;
  cost_cents: number;
  duration_ms: number | null;
  started_at: number | null;
  ended_at: number | null;
}

export interface JobReportTurn {
  task_id: string;
  stage_ordinal: number | null;
  cost_cents: number;
  input_tokens: number;
  output_tokens: number;
  at: number;
}

export interface JobReportToolCall {
  tool: string;
  count: number;
}

export interface JobReportEventTally {
  kind: string;
  count: number;
}

export interface JobReportSpecChange {
  kind: string;
  filename: string | null;
  count: number;
  last_at: number;
}

export interface JobReportResult {
  job_id: JobId;
  status: string;
  stop_reason: string | null;
  cost_cents: number;
  cost_cap_cents: number;
  started_at: number | null;
  ended_at: number | null;
  wall_clock_ms: number | null;
  stages: JobReportStage[];
  turns: JobReportTurn[];
  tool_calls: JobReportToolCall[];
  event_tally: JobReportEventTally[];
  spec_changes: JobReportSpecChange[];
}

export interface StopJobArgs {
  job_id: JobId;
}

// Move a Running / AwaitingReview job to Paused. The runner is
// cancelled at the next await boundary; the captured per-stage
// Stage.session_id is the resume handle for the next resume_job
// call. Distinct from stop_job: pause is "I'll come back."
export interface PauseJobArgs {
  job_id: JobId;
}

export interface StartJobArgs {
  job_id: JobId;
}

// A0 — intra-stage session continuation. Re-queues a terminal-but-
// recoverable job (Stopped or Failed) so the driver picks it up
// again, reusing the captured per-stage session id so the next
// claude task passes `--continue` instead of starting fresh. Both
// caps are additive on the existing job; null leaves them as-is.
export interface ResumeJobArgs {
  job_id: JobId;
  additional_cost_cap_cents?: number | null;
  additional_wall_clock_cap_ms?: number | null;
  // Operator-supplied free-text comment threaded into the next
  // stage's prompt under an `# Operator comment` heading. Same
  // envelope auto-bypass uses, so the model parses one form, not
  // two. Empty string is treated as null by the runtime.
  next_stage_comment?: string | null;
}

export interface RerunJobArgs {
  source_job_id: JobId;
}

// Recovery hatch for a stuck Queued (driver gave up after retry
// budget exhausted) or a terminal Failed / Stopped row that the
// operator wants to edit before resubmitting. Refused server-side
// for Running / Paused / AwaitingReview / Completed — those have
// their own transitions (stop_job / pause_job / resume_job /
// rerun_job).
export interface ResetJobArgs {
  job_id: JobId;
}

export interface UpdateJobArgs {
  job_id: JobId;
  runner?: string | null;
  model?: string | null;
  permission_mode?: string | null;
  effort?: string | null;
  cost_cap_cents?: number | null;
  wall_clock_cap_ms?: number | null;
  branch?: string | null;
}

export interface DeleteJobArgs {
  job_id: JobId;
}

// Mid-life policy change for Surface F. The runtime accepts the call
// when the job is `Draft | Queued | Paused | Stopped | Failed |
// Completed` and refuses with `Conflict` on `Running |
// AwaitingReview` — the operator pauses, sets, resumes. `policy =
// null` clears the policy and restores halt-on-failure. The Rust
// counterpart lands alongside the JobPage policy-badge modal that
// calls this; until then the UI button surfaces the conflict as an
// inline error message.
export interface SetJobPolicyArgs {
  job_id: JobId;
  auto_bypass_policy: AutoBypassPolicy | null;
}

export interface GcWorktreesArgs {
  older_than_ms: number | null;
  job_ids: JobId[] | null;
  dry_run: boolean;
}

export interface GcWorktreeEntry {
  job_id: JobId | null;
  path: string;
  size_bytes: number;
  mtime_ms: number | null;
  removed: boolean;
  error: string | null;
}

export interface GcWorktreesResult {
  entries: GcWorktreeEntry[];
  total_size_bytes: number;
  removed_count: number;
  root: string | null;
}

export type EventFilter =
  | { scope: "all" }
  | { scope: "job"; job_id: JobId };

export type Since = EventCursor | null;

// Filesystem RPC surface. Provisional: hand-mirrored from the
// forthcoming Rust additions in `codeless-rpc::methods::fs_*` that
// `codeless-runtime` will route to `codeless-adapters-host`'s
// worktree-scoped FS layer. Stage 15 of the UI conversion loop
// replaces this section with the specta-generated snapshot.

export interface FsReadFileArgs {
  path: string;
  byte_limit: number | null;
}

export interface FsWriteFileArgs {
  path: string;
  content: string;
  create_parents: boolean;
}

export interface FsCreateFileArgs {
  path: string;
  content: string | null;
  overwrite: boolean;
}

export interface FsCreateDirArgs {
  path: string;
  recursive: boolean;
}

export interface FsReadDirArgs {
  path: string;
}

export interface FsReadDirResult {
  entries: FsEntry[];
}

export interface FsSearchArgs {
  root: string;
  query: string;
  case_sensitive: boolean;
  max_results: number | null;
  glob: string | null;
}

export interface FsSearchResult {
  hits: FsGrepHit[];
  truncated: boolean;
}

export interface FsGlobArgs {
  root: string;
  pattern: string;
  max_results: number | null;
}

export interface FsGlobResult {
  hits: FsGlobHit[];
  truncated: boolean;
}

export interface FsMoveArgs {
  from: string;
  to: string;
  overwrite: boolean;
}

export interface FsDeleteArgs {
  path: string;
  recursive: boolean;
}

export interface FsCwdResult {
  path: string;
}

// Job-file surface — the four RPCs that back the Spec pane.
// Mirrored from `codeless-rpc::methods` and gated behind a known
// `template_yaml`; non-template jobs surface `InvalidArgument`.
// `layout` is `"directory" | "flat" | "none"` so the UI can render
// the legacy-flat hint when migration hasn't happened yet.

export interface ListJobFilesArgs {
  job_id: JobId;
}

export interface JobFileEntry {
  name: string;
  is_template: boolean;
  is_scope: boolean;
  is_workflow: boolean;
}

export interface ListJobFilesResult {
  entries: JobFileEntry[];
  layout: string;
  directory_path: string | null;
}

export interface ReadJobFileArgs {
  job_id: JobId;
  filename: string;
}

export interface ReadJobFileResult {
  content: string;
}

export interface WriteJobFileArgs {
  job_id: JobId;
  filename: string;
  content: string;
}

export interface WriteJobFileResult {
  name: string;
}

export interface DeleteJobFileArgs {
  job_id: JobId;
  filename: string;
}

// Spec replacement — the stages-CRUD editor saves the whole
// template.yaml back through this RPC. Validated server-side via the
// same JobTemplate parser the runner uses; renames are rejected with
// `conflict` (the job dir is addressed by name and a rename would
// orphan the existing SCOPE/WORKFLOW/extras).

export interface UpdateJobTemplateArgs {
  job_id: JobId;
  template_yaml: string;
}

export interface UpdateJobTemplateResult {
  name: string;
}

// Handover seeding — JOB-MODEL.md says handover.md lives in the
// worktree (per-run), not the source repo (per-job). The UI uses
// this to create one from scratch when a runner hasn't yet written
// one. Jobs without a `worktree_path` get `conflict` on the wire.

export interface WriteHandoverArgs {
  job_id: JobId;
  handover: Handover;
  // Optional stage id. When omitted, the runtime writes to the job's
  // highest-ordinal stage (the current "active" stage).
  stage_id?: StageId | null;
}

export interface WriteHandoverResult {
  path: string;
}

// Secrets RPC surface. Provisional: hand-mirrored from the forthcoming
// `codeless-rpc::methods::secrets_*`. Provider keys live in the
// single-tenant secrets file managed by `codeless-adapters-host`
// (see SCOPE.md "Rule 5 — Single-tenant trust boundary"). `get`
// returns the raw secret; the UI is trusted with it inside the
// trust boundary. List returns metadata only — never values.

export interface SecretsSetArgs {
  provider: string;
  value: string;
}

export interface SecretsGetArgs {
  provider: string;
}

export interface SecretsListEntry {
  provider: string;
}

export interface SecretsListResult {
  entries: SecretsListEntry[];
}

export interface SecretsRmArgs {
  provider: string;
}

// Persona RPC surface (agent-personas stage 7). Personas live in
// SQLite (`personas` table, migration 0011); this is the wire the
// UI's `ai-agents` KV store mirrors. The KV stays as a cache so a
// brief outage of the RPC does not lose the persona dropdown — but
// the runtime is the source of truth (R4).
export interface Persona {
  id: string;
  name: string;
  description: string;
  icon: string;
  instructions: string;
  use_for_jobs: boolean;
  default_model: string | null;
  allowed_subagents: string[];
  default_snippets: string[];
  built_in: boolean;
  created_at: number;
  updated_at: number;
}

export interface ListPersonasArgs {}
export interface ListPersonasResult {
  personas: Persona[];
}
export interface GetPersonaArgs {
  id: string;
}
export interface UpsertPersonaArgs {
  id: string;
  name: string;
  description: string;
  icon: string;
  instructions: string;
  use_for_jobs: boolean;
  default_model?: string | null;
  allowed_subagents: string[];
  default_snippets?: string[];
}
export interface DeletePersonaArgs {
  id: string;
}

export interface RpcMethodMap {
  add_repo: { args: AddRepoArgs; result: Repo };
  remove_repo: { args: RemoveRepoArgs; result: null };
  list_repos: { args: Record<string, never>; result: ListReposResult };
  submit_job: { args: SubmitJobArgs; result: Job };
  get_job: { args: GetJobArgs; result: Job };
  list_jobs: { args: ListJobsArgs; result: ListJobsResult };
  list_stages: { args: ListStagesArgs; result: ListStagesResult };
  job_report: { args: JobReportArgs; result: JobReportResult };
  stop_job: { args: StopJobArgs; result: null };
  stop_active: { args: StopActiveArgs; result: StopActiveResult };
  pause_job: { args: PauseJobArgs; result: null };
  start_job: { args: StartJobArgs; result: Job };
  resume_job: { args: ResumeJobArgs; result: Job };
  // Manual recovery hatch — moves a wedged Queued / Failed / Stopped
  // row back to Draft so the operator can edit and re-start. The
  // captured worktree is reaped best-effort; the button surfaces this
  // RPC only when the driver could not recover on its own.
  reset_job: { args: ResetJobArgs; result: Job };
  rerun_job: { args: RerunJobArgs; result: Job };
  update_job: { args: UpdateJobArgs; result: Job };
  set_job_policy: { args: SetJobPolicyArgs; result: Job };
  delete_job: { args: DeleteJobArgs; result: null };
  gc_worktrees: { args: GcWorktreesArgs; result: GcWorktreesResult };
  job_diff: { args: JobDiffArgs; result: JobDiffResult };

  fs_read_file: { args: FsReadFileArgs; result: FsReadResult };
  fs_write_file: { args: FsWriteFileArgs; result: null };
  fs_create_file: { args: FsCreateFileArgs; result: null };
  fs_create_dir: { args: FsCreateDirArgs; result: null };
  fs_read_dir: { args: FsReadDirArgs; result: FsReadDirResult };
  fs_search: { args: FsSearchArgs; result: FsSearchResult };
  fs_glob: { args: FsGlobArgs; result: FsGlobResult };
  fs_move: { args: FsMoveArgs; result: null };
  fs_delete: { args: FsDeleteArgs; result: null };
  fs_stat: { args: FsStatArgs; result: FsStatResult };
  fs_cwd: { args: Record<string, never>; result: FsCwdResult };

  list_job_files: { args: ListJobFilesArgs; result: ListJobFilesResult };
  read_job_file: { args: ReadJobFileArgs; result: ReadJobFileResult };
  write_job_file: { args: WriteJobFileArgs; result: WriteJobFileResult };
  delete_job_file: { args: DeleteJobFileArgs; result: null };
  update_job_template: {
    args: UpdateJobTemplateArgs;
    result: UpdateJobTemplateResult;
  };
  write_handover: { args: WriteHandoverArgs; result: WriteHandoverResult };

  secrets_set: { args: SecretsSetArgs; result: null };
  secrets_get: { args: SecretsGetArgs; result: string | null };
  secrets_list: { args: Record<string, never>; result: SecretsListResult };
  secrets_rm: { args: SecretsRmArgs; result: null };

  shell_run: { args: ShellRunArgs; result: ShellCommandOutput };
  shell_session_open: { args: ShellSessionOpenArgs; result: number };
  shell_session_run: { args: ShellSessionRunArgs; result: ShellSessionRunOutput };
  shell_session_close: { args: ShellSessionCloseArgs; result: null };
  shell_bg_spawn: { args: ShellBgSpawnArgs; result: number };
  shell_bg_logs: { args: ShellBgLogsArgs; result: ShellBgLogChunk };
  shell_bg_kill: { args: ShellBgKillArgs; result: null };
  shell_bg_list: { args: Record<string, never>; result: { entries: ShellBgEntry[] } };

  list_reviews: { args: ListReviewsArgs; result: ListReviewsResult };
  approve_review: { args: ReviewActionArgs; result: Review };
  comment_review: { args: CommentReviewArgs; result: Review };

  // Scope-patch resolution surface. The runtime author resolves identity
  // from repo-local `git config user.{name,email}`; the UI sends no
  // author args. All three RPCs return the same `ScopePatchActionResult`
  // wire shape — including `AlreadyResolved` on a re-call from a stale
  // window — so the patch inbox can fold the response uniformly.
  approve_scope_patch: {
    args: ApproveScopePatchArgs;
    result: ScopePatchActionResult;
  };
  reject_scope_patch: {
    args: RejectScopePatchArgs;
    result: ScopePatchActionResult;
  };
  edit_scope_patch: {
    args: EditScopePatchArgs;
    result: ScopePatchActionResult;
  };
  // Undo a prior approval by SHA. Wired only from the inbox's 10s
  // post-approval undo toast; reverts beyond that window happen
  // through plain `git revert` out-of-band.
  revert_scope_patch: {
    args: RevertScopePatchArgs;
    result: RevertScopePatchResult;
  };
  // Snapshot the unresolved patch queue across one or all repos.
  // Powers Surface C (cross-workspace worklist); `repo_id = null`
  // walks every repo. Runtime returns entries newest-first by
  // `proposed_at`, with legacy undated entries trailing. Filters,
  // group-by, and 14-day-decay layering live client-side so the user
  // can flip "show everything" without a round-trip.
  list_proposed_patches: {
    args: ListProposedPatchesArgs;
    result: ListProposedPatchesResult;
  };
  stop_review: { args: ReviewActionArgs; result: Review };

  agent_chat: { args: AgentChatArgs; result: AgentChatResult };
  upload_chat_attachment: {
    args: UploadChatAttachmentArgs;
    result: UploadChatAttachmentResult;
  };
  cancel_chat_task: { args: CancelChatTaskArgs; result: null };

  // Assistant surface (DOCS/ASSISTANT-SCOPE.md). Persistence-only in
  // stage 5; later stages add listMessages / subscribe / actions.
  list_assistant_threads: {
    args: ListAssistantThreadsArgs;
    result: ListAssistantThreadsResult;
  };
  create_assistant_thread: {
    args: CreateAssistantThreadArgs;
    result: AssistantThread;
  };
  delete_assistant_thread: {
    args: DeleteAssistantThreadArgs;
    result: null;
  };
  upload_assistant_attachment: {
    args: UploadAssistantAttachmentArgs;
    result: UploadAssistantAttachmentResult;
  };
  list_assistant_messages: {
    args: ListAssistantMessagesArgs;
    result: ListAssistantMessagesResult;
  };
  append_assistant_message: {
    args: AppendAssistantMessageArgs;
    result: AppendAssistantMessageResult;
  };
  confirm_assistant_action: {
    args: ConfirmAssistantActionArgs;
    result: ConfirmAssistantActionResult;
  };
  cancel_assistant_action: {
    args: CancelAssistantActionArgs;
    result: CancelAssistantActionResult;
  };

  list_personas: { args: ListPersonasArgs; result: ListPersonasResult };
  get_persona: { args: GetPersonaArgs; result: Persona };
  upsert_persona: { args: UpsertPersonaArgs; result: Persona };
  delete_persona: { args: DeletePersonaArgs; result: null };
}

export type {
  AgentChatArgs,
  AgentChatResult,
  CancelChatTaskArgs,
  StopActiveArgs,
  StopActiveResult,
  UploadChatAttachmentArgs,
  UploadChatAttachmentResult,
  AssistantThread,
  AssistantMessage,
  ListAssistantThreadsArgs,
  ListAssistantThreadsResult,
  CreateAssistantThreadArgs,
  DeleteAssistantThreadArgs,
  UploadAssistantAttachmentArgs,
  UploadAssistantAttachmentResult,
  ListAssistantMessagesArgs,
  ListAssistantMessagesResult,
  AppendAssistantMessageArgs,
  AppendAssistantMessageResult,
  AssistantAction,
  AssistantActionCard,
  AssistantActionStatus,
  ConfirmAssistantActionArgs,
  ConfirmAssistantActionResult,
  CancelAssistantActionArgs,
  CancelAssistantActionResult,
};

// Review RPC surface. `ListReviewsArgs`, `ListReviewsResult`,
// `ApproveReviewArgs`, `CommentReviewArgs`, and `StopReviewArgs` are
// generated from `codeless-rpc::methods` and re-exported via `./wire`.
// `ReviewActionArgs` is a UI-side alias used by the panel because
// approve and stop share the same single-field shape.

import type {
  ApproveReviewArgs,
  CommentReviewArgs,
  ListReviewsArgs,
  ListReviewsResult,
  StopReviewArgs,
} from "./wire";

export type {
  ApproveReviewArgs,
  CommentReviewArgs,
  ListReviewsArgs,
  ListReviewsResult,
  StopReviewArgs,
};
export type ReviewActionArgs = ApproveReviewArgs;

// Shell RPC surface. Provisional: hand-mirrored from the forthcoming
// `codeless-rpc::methods::shell_*` that `codeless-runtime` routes to
// `codeless-adapters-host`'s process-spawn + PTY layer. The
// interactive PTY *stream* itself does not flow through here — it
// uses the SCOPE.md-mandated PTY WebSocket. These methods cover the
// one-shot run, the cwd-preserving session, and the long-running
// background-process surface.

export interface ShellRunArgs {
  command: string;
  cwd: string | null;
  timeout_secs: number | null;
}

export interface ShellSessionOpenArgs {
  cwd: string | null;
}

export interface ShellSessionRunArgs {
  id: number;
  command: string;
  cwd: string | null;
  timeout_secs: number | null;
}

export interface ShellSessionCloseArgs {
  id: number;
}

export interface ShellBgSpawnArgs {
  command: string;
  cwd: string | null;
}

export interface ShellBgLogsArgs {
  handle: number;
  since_offset: number | null;
}

export interface ShellBgKillArgs {
  handle: number;
}

export type RpcMethod = keyof RpcMethodMap;
export type RpcArgs<M extends RpcMethod> = RpcMethodMap[M]["args"];
export type RpcResultOf<M extends RpcMethod> = RpcMethodMap[M]["result"];
