export type {
  RpcClient,
  SseConnectionState,
  SseConnectionStatus,
} from "./client";
export { HttpSseClient, type HttpSseClientConfig } from "./http-sse-client";
export { TauriIpcClient } from "./tauri-ipc-client";
export { RpcError, type RpcErrorKind } from "./error";
export { RpcProvider, useRpc } from "./provider";
export { isViteDevPort, readBaseUrl, readToken } from "./config";
export { MockRpcClient } from "./mock-client";
export {
  useRepos,
  useJobs,
  useJob,
  useReviews,
  useEventStream,
  useEventStreamWithState,
  type QueryState,
} from "./hooks";
export type {
  AddRepoArgs,
  DeleteJobFileArgs,
  EventFilter,
  GcWorktreeEntry,
  GcWorktreesArgs,
  GcWorktreesResult,
  GetJobArgs,
  JobFileEntry,
  JobReportArgs,
  JobReportEventTally,
  JobReportResult,
  JobReportStage,
  JobReportToolCall,
  JobReportTurn,
  ListJobFilesArgs,
  ListJobFilesResult,
  ListJobsArgs,
  ListJobsResult,
  ListReposResult,
  ListStagesArgs,
  ListStagesResult,
  StageRollup,
  ReadJobFileArgs,
  ReadJobFileResult,
  RemoveRepoArgs,
  RerunJobArgs,
  RpcArgs,
  RpcMethod,
  RpcMethodMap,
  RpcResultOf,
  Since,
  StartJobArgs,
  StopJobArgs,
  SubmitJobArgs,
  UpdateJobTemplateArgs,
  UpdateJobTemplateResult,
  WriteHandoverArgs,
  WriteHandoverResult,
  WriteJobFileArgs,
  WriteJobFileResult,
} from "./methods";
export type * from "./wire";
