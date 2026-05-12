export type { RpcClient } from "./client";
export { HttpSseClient, type HttpSseClientConfig } from "./http-sse-client";
export { RpcError, type RpcErrorKind } from "./error";
export { RpcProvider, useRpc } from "./provider";
export { readBaseUrl, readToken } from "./config";
export { MockRpcClient } from "./mock-client";
export {
  useRepos,
  useJobs,
  useJob,
  useEventStream,
  type QueryState,
} from "./hooks";
export type {
  AddRepoArgs,
  EventFilter,
  GetJobArgs,
  ListJobsArgs,
  ListJobsResult,
  ListReposResult,
  RemoveRepoArgs,
  RpcArgs,
  RpcMethod,
  RpcMethodMap,
  RpcResultOf,
  Since,
  StopJobArgs,
  SubmitJobArgs,
} from "./methods";
export type * from "./wire";
