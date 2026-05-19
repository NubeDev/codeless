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
  PausePoint,
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
  BindChatThreadArgs,
  ChatBinding,
  ChatMessage,
  ChatRole,
  ChatTransport,
  ListJobMessagesArgs,
  ListJobMessagesResult,
  MessageId,
  PostJobMessageArgs,
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
  AssistantAttachmentCard,
  AssistantAttachmentCardItem,
  AttachmentRef,
  ConfirmAssistantActionArgs,
  ConfirmAssistantActionResult,
  CancelAssistantActionArgs,
  CancelAssistantActionResult,
  ListPluginsResult,
  PluginListEntry,
  PluginUiContribution,
  PluginUiExposeEntry,
  SetAssistantThreadModeArgs,
  SetAssistantThreadModeResult,
} from "./wire";
import type {
  AttachWorkspaceArgs,
  AttachWorkspaceResult,
  DetachWorkspaceArgs,
  ListWorkspacesResult,
  ValidateWorkspacePathArgs,
  ValidateWorkspacePathResult,
} from "./wire";

export type {
  AttachWorkspaceArgs,
  AttachWorkspaceResult,
  AttachedWorkspace,
  DetachPolicy,
  DetachWorkspaceArgs,
  ListWorkspacesResult,
  ValidateWorkspacePathArgs,
  ValidateWorkspacePathResult,
  WorkspaceError,
  WorkspaceProblem,
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
    // Forward-advance signal: when set together with `status: failed`,
    // an operator (or auto-bypass policy) advanced past the failure
    // instead of halting. The UI uses this to switch the failed glyph
    // from `!`-in-destructive to `~`-in-muted so a bypassed-after-
    // failure row reads as "recovered, keep watching" not "halted".
    bypassed_at?: number | null;
    // Operator (or policy) free-text reason for the bypass; rendered
    // in the stage tooltip so the audit trail names *why* the bypass
    // happened without a second RPC.
    bypassed_reason?: string | null;
    // Coarse machine-readable classification of *why* this stage
    // ended `failed`. Paired with `failure_detail` on the stage
    // tooltip so the operator sees both the bucket and the verbatim
    // reason string.
    failure_class?: string | null;
    // Short human-readable failure description (one line, ~200 chars).
    // Surfaced under the bypassed tooltip so the recovered-past row
    // still carries the reason the rail failed.
    failure_detail?: string | null;
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

// Mirrors the wire shape of `codeless_types::pause_point` — kept
// hand-mirrored here for now because `methods.ts` is the call-table's
// type contract and the UI imports its `PausePoint*` from `./wire`
// already (the specta-generated module). We re-export those types via
// `./wire` to keep one source of truth at the wire boundary.
export interface ListScheduledPausePointsArgs {
  job_id: JobId;
}

export interface ListScheduledPausePointsResult {
  // The wire is an ordered list; preserve the YAML order so the divider
  // chips render in the same sequence the operator wrote.
  points: PausePoint[];
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

// Operator-explicit escape from a `StopReason::ReviewPreCheck` halt.
// The diff-verify pre-check is deterministic against the prior
// stage's handover, so a plain `resume_job` re-fails identically;
// this RPC flips a one-shot server-side flag the runner consumes
// just before the gate. `comment` is required and non-empty — it
// lands on `pending_operator_comment` so the model sees why the
// gate was bypassed and so the audit log has the operator's
// justification.
export interface OverridePreCheckAndResumeArgs {
  job_id: JobId;
  comment: string;
  additional_cost_cap_cents?: number | null;
  additional_wall_clock_cap_ms?: number | null;
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
  policy: AutoBypassPolicy | null;
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
  | { scope: "job"; job_id: JobId }
  | { scope: "repo"; repo_id: RepoId }
  | { scope: "library" };

export type Since = EventCursor | null;

// Filesystem RPC surface. The arg-types for the calls that exist
// server-side (`fs_read_dir`, `fs_read_file`, `fs_write_file`,
// `fs_create_file`, `fs_create_dir`, `fs_stat`, `fs_move`, `fs_delete`,
// `fs_cwd`) are now generated by specta and live in `./wire`; they
// each carry the `repo_id: RepoId` that the runtime resolves to the
// attached workspace's `fs_root_canonical`. `fs_search` and `fs_glob`
// are stub methods backed only by the mock client today — they are
// kept hand-mirrored here and carry `repo_id` for parity so the call
// sites do not branch on which transport they happen to be running
// against.

import type {
  FsCreateDirArgs,
  FsCreateFileArgs,
  FsCwdArgs,
  FsCwdResult,
  FsDeleteArgs,
  FsMoveArgs,
  FsReadDirArgs,
  FsReadDirResult,
  FsReadFileArgs,
  FsWriteFileArgs,
} from "./wire";

export type {
  FsCreateDirArgs,
  FsCreateFileArgs,
  FsCwdArgs,
  FsCwdResult,
  FsDeleteArgs,
  FsMoveArgs,
  FsReadDirArgs,
  FsReadDirResult,
  FsReadFileArgs,
  FsWriteFileArgs,
};

export interface FsSearchArgs {
  repo_id: RepoId;
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
  repo_id: RepoId;
  root: string;
  pattern: string;
  max_results: number | null;
}

export interface FsGlobResult {
  hits: FsGlobHit[];
  truncated: boolean;
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
  list_scheduled_pause_points: {
    args: ListScheduledPausePointsArgs;
    result: ListScheduledPausePointsResult;
  };
  job_report: { args: JobReportArgs; result: JobReportResult };
  stop_job: { args: StopJobArgs; result: null };
  stop_active: { args: StopActiveArgs; result: StopActiveResult };
  pause_job: { args: PauseJobArgs; result: null };
  start_job: { args: StartJobArgs; result: Job };
  resume_job: { args: ResumeJobArgs; result: Job };
  override_pre_check_and_resume: {
    args: OverridePreCheckAndResumeArgs;
    result: Job;
  };
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
  fs_cwd: { args: FsCwdArgs; result: FsCwdResult };

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

  // Per-Job chat substrate (DOCS/JOB-CHAT.md). `post_job_message`
  // inserts a row into `chat_messages` and fans out via the
  // `chat-message-appended` event; `list_job_messages` pages the
  // history newest-first by `created_at`; `bind_chat_thread` ties an
  // external `(transport, channel, thread)` to a Job for the
  // Telegram / Slack adapter inbound path. The web UI never calls
  // `bind_chat_thread` itself but the typed entry stays here so
  // every surface compiles against the same shape.
  post_job_message: { args: PostJobMessageArgs; result: ChatMessage };
  list_job_messages: {
    args: ListJobMessagesArgs;
    result: ListJobMessagesResult;
  };
  bind_chat_thread: { args: BindChatThreadArgs; result: ChatBinding };

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
  // Per-thread filesystem-tool permission posture (job
  // `assistant-fs-tools`). The mode is the server's authoritative
  // value (R4); the UI dropdown calls this method and re-reads the
  // thread row to display the result — it never trusts a local
  // optimistic cache.
  set_assistant_thread_mode: {
    args: SetAssistantThreadModeArgs;
    result: SetAssistantThreadModeResult;
  };

  // Workspace-attach surface (DOCS/WORKSPACE-ATTACH.md M3). The
  // server-side RPCs already exist (M2); this is the UI-side
  // pickup so callers can `client.call("attach_workspace", ...)`
  // through the typed boundary. `PathPicker` injection and the
  // Settings -> Workspaces UI land in later stages.
  attach_workspace: { args: AttachWorkspaceArgs; result: AttachWorkspaceResult };
  detach_workspace: { args: DetachWorkspaceArgs; result: null };
  list_workspaces: {
    args: Record<string, never>;
    result: ListWorkspacesResult;
  };
  validate_workspace_path: {
    args: ValidateWorkspacePathArgs;
    result: ValidateWorkspacePathResult;
  };

  list_personas: { args: ListPersonasArgs; result: ListPersonasResult };
  get_persona: { args: GetPersonaArgs; result: Persona };
  upsert_persona: { args: UpsertPersonaArgs; result: Persona };
  delete_persona: { args: DeletePersonaArgs; result: null };

  // Plugin substrate — UI federation host wiring
  // (DOCS/plugins/PLUGIN-UI-FEDERATION.md § Host wiring). Returns the
  // enabled-plugin projection the host shell reads at boot to
  // register MF remotes and resolve <PluginSlot/> contributors. The
  // server-side implementation lands with the rest of the
  // `[[runtimes]]` + `[contributes.ui]` manifest parsing; until then
  // the host catches the "method not found" error and degrades to
  // the empty list — every slot site renders its fallback.
  list_plugins: { args: Record<string, never>; result: ListPluginsResult };

  // Job export/import surface (DOCS/SCOPE-JOB-EXPORT.md). The bundle
  // lives at a server-side path; UI streams the bytes via `fs_read_file`
  // for browser-shell downloads. `inspect_job_bundle` returns the
  // manifest without touching SQLite so the Import dialog can preview
  // the bundle before the user commits.
  export_job: { args: ExportJobArgs; result: ExportJobResult };
  import_job: { args: ImportJobArgs; result: ImportJobResult };
  inspect_job_bundle: {
    args: InspectJobBundleArgs;
    result: InspectJobBundleResult;
  };
}

export interface ExportJobArgs {
  job_id: JobId;
  output_path: string;
  include_artifacts: boolean;
}

export interface ExportJobResult {
  output_path: string;
  bytes_written: number;
  run_count: number;
  event_count: number;
}

// `Refuse` is the default; the destination workspace's existing Job
// row wins and the importer surfaces the collision. `Suffix` renames
// the incoming Job with a numeric suffix. `Replace` drops the
// existing rows but leaves the on-disk worktree for inspection.
export type ImportConflictPolicy = "Refuse" | "Suffix" | "Replace";

export interface ImportJobArgs {
  workspace_id: RepoId;
  bundle_path: string;
  rename_to: string | null;
  on_conflict: ImportConflictPolicy;
}

// Non-fatal mismatches surfaced by the importer. The wire shape is
// loose on purpose — the server's `ImportWarning` may grow new
// variants and the UI banner renders whatever `message` it gets
// without needing a typed discriminant for each kind.
export interface ImportWarning {
  kind: string;
  message: string;
}

export interface ImportJobResult {
  job_id: JobId;
  imported_name: string;
  run_count: number;
  warnings: ImportWarning[];
}

export interface InspectJobBundleArgs {
  bundle_path: string;
}

export interface JobBundleManifest {
  schema_version: number;
  exported_at: string;
  exporter: {
    codeless_version: string;
    host_os: string;
  };
  source: {
    workspace_name: string;
    repo_url: string;
    repo_commit: string;
    job_name: string;
    job_id: string;
    run_count: number;
  };
  content: {
    has_handover: boolean;
    note_count: number;
    total_events: number;
    includes_artifacts: boolean;
  };
}

// `local_warnings` describes mismatches the inspector can detect
// without writing anything — e.g. the destination workspace's HEAD
// SHA differs from `manifest.source.repo_commit`, or no workspace
// is currently active. The dialog renders these as a banner above
// the conflict-policy selector so the user sees the risk before
// committing to the import.
export interface InspectJobBundleResult {
  manifest: JobBundleManifest;
  bytes: number;
  local_warnings: ImportWarning[];
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
  AssistantAttachmentCard,
  AssistantAttachmentCardItem,
  AttachmentRef,
  ConfirmAssistantActionArgs,
  ConfirmAssistantActionResult,
  CancelAssistantActionArgs,
  CancelAssistantActionResult,
  BindChatThreadArgs,
  ChatBinding,
  ChatMessage,
  ChatRole,
  ChatTransport,
  ListJobMessagesArgs,
  ListJobMessagesResult,
  MessageId,
  PostJobMessageArgs,
  ListPluginsResult,
  PluginListEntry,
  PluginUiContribution,
  PluginUiExposeEntry,
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
