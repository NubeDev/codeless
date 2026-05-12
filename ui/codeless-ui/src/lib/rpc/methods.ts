// RPC method args + results — hand-mirrored from
// `codeless/crates/codeless-rpc/src/methods.rs`. Specta currently
// generates only the wire types in `wire.ts`; method-arg structs are
// not yet in the codegen output. When that lands, replace this file.

import type {
  EventCursor,
  FsEntry,
  FsGlobHit,
  FsGrepHit,
  FsReadResult,
  GitAuth,
  Job,
  JobId,
  Repo,
  RepoId,
  Review,
  ReviewId,
  ShellBgEntry,
  ShellBgLogChunk,
  ShellCommandOutput,
  ShellSessionRunOutput,
  StageId,
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
  cost_cap_cents: number;
  wall_clock_cap_ms: number;
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

export interface StopJobArgs {
  job_id: JobId;
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

export interface RpcMethodMap {
  add_repo: { args: AddRepoArgs; result: Repo };
  remove_repo: { args: RemoveRepoArgs; result: null };
  list_repos: { args: Record<string, never>; result: ListReposResult };
  submit_job: { args: SubmitJobArgs; result: Job };
  get_job: { args: GetJobArgs; result: Job };
  list_jobs: { args: ListJobsArgs; result: ListJobsResult };
  stop_job: { args: StopJobArgs; result: null };

  fs_read_file: { args: FsReadFileArgs; result: FsReadResult };
  fs_write_file: { args: FsWriteFileArgs; result: null };
  fs_create_file: { args: FsCreateFileArgs; result: null };
  fs_create_dir: { args: FsCreateDirArgs; result: null };
  fs_read_dir: { args: FsReadDirArgs; result: FsReadDirResult };
  fs_search: { args: FsSearchArgs; result: FsSearchResult };
  fs_glob: { args: FsGlobArgs; result: FsGlobResult };
  fs_move: { args: FsMoveArgs; result: null };
  fs_delete: { args: FsDeleteArgs; result: null };
  fs_cwd: { args: Record<string, never>; result: FsCwdResult };

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
  stop_review: { args: ReviewActionArgs; result: Review };
}

// Review RPC surface. Mirrored from the Phase 2c additions on
// `codeless-rpc::methods` (`list_reviews`, `approve_review`,
// `comment_review`, `stop_review`). The state machine lives in
// `codeless-runtime` and matches the `ReviewStatus` enum in wire.ts.

export interface ListReviewsArgs {
  // Filter by job. `null` returns every pending/recent review across
  // the host — the dashboard uses that; per-job panels narrow it.
  job_id: JobId | null;
  stage_id: StageId | null;
  pending_only: boolean;
}

export interface ListReviewsResult {
  reviews: Review[];
}

export interface ReviewActionArgs {
  review_id: ReviewId;
}

export interface CommentReviewArgs {
  review_id: ReviewId;
  comment: string;
}

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
