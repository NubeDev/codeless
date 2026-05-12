// RPC method args + results — hand-mirrored from
// `codeless/crates/codeless-rpc/src/methods.rs`. Specta currently
// generates only the wire types in `wire.ts`; method-arg structs are
// not yet in the codegen output. When that lands, replace this file.

import type {
  EventCursor,
  GitAuth,
  Job,
  JobId,
  Repo,
  RepoId,
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

export interface RpcMethodMap {
  add_repo: { args: AddRepoArgs; result: Repo };
  remove_repo: { args: RemoveRepoArgs; result: null };
  list_repos: { args: Record<string, never>; result: ListReposResult };
  submit_job: { args: SubmitJobArgs; result: Job };
  get_job: { args: GetJobArgs; result: Job };
  list_jobs: { args: ListJobsArgs; result: ListJobsResult };
  stop_job: { args: StopJobArgs; result: null };
}

export type RpcMethod = keyof RpcMethodMap;
export type RpcArgs<M extends RpcMethod> = RpcMethodMap[M]["args"];
export type RpcResultOf<M extends RpcMethod> = RpcMethodMap[M]["result"];
