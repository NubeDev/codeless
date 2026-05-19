//! Transport-agnostic RPC surface. Every client transport — SSE/REST in
//! the browser, Tauri IPC on desktop, in-process in the CLI — adapts to
//! the same `RpcServer` trait. See `DOCS/SCOPE.md` "Rule 1 — One
//! transport interface, many implementations".
//!
//! No I/O assumptions live here: `async-trait` makes the trait
//! object-safe, `futures-core::Stream` keeps the subscription type free
//! of any executor, and `serde` lets every argument/result round-trip
//! over whatever wire the chosen transport uses.

pub mod error;
pub mod methods;
pub mod server;
pub mod subscribe;

pub use codeless_types::{
    AdapterError, AttachWorkspaceArgs, AttachWorkspaceResult, AttachedWorkspace, ChatAdapterKind,
    ChatAdapterRow, ChatAdapterSecretProblem, DetachPolicy, DetachWorkspaceArgs,
    ListChatAdaptersResult, ListRunnersResult, ListWorkspacesResult, Persona, RestartServerArgs,
    RestartServerResult, RunnerRow, SetChatAdapterEnabledArgs, SetRunnerEnabledArgs,
    ValidateChatAdapterSecretsArgs, ValidateChatAdapterSecretsResult, ValidateWorkspacePathArgs,
    ValidateWorkspacePathResult, WorkspaceError, WorkspaceProblem,
};
pub use error::{RpcError, RpcResult};
pub use methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, ApproveScopePatchArgs, BindChatThreadArgs,
    CancelAssistantActionArgs, CancelAssistantActionResult, CancelChatTaskArgs, ChatAttachmentRef,
    ChatContext, ChatMode, ClaudeStatus, CommentReviewArgs, ConfirmAssistantActionArgs,
    ConfirmAssistantActionResult, CreateAssistantThreadArgs, DeleteAssistantThreadArgs,
    DeleteJobArgs, DeleteJobFileArgs, DeletePersonaArgs, DraftJobFromConversationArgs,
    EditScopePatchArgs, FsCreateDirArgs, FsCreateFileArgs, FsCwdArgs, FsCwdResult, FsDeleteArgs,
    FsMoveArgs, FsReadDirArgs, FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs,
    FsStatResult, FsWriteFileArgs, GcWorktreeEntry, GcWorktreesArgs, GcWorktreesResult,
    GetChatBindingArgs, GetChatBindingResult, GetJobArgs, GetPersonaArgs, JobContextRef,
    JobDiffArgs, JobDiffFile, JobDiffResult, JobFileEntry, JobReportArgs, JobReportEventTally,
    JobReportResult, JobReportSpecChange, JobReportStage, JobReportToolCall, JobReportTurn,
    ListAssistantMessagesArgs, ListAssistantMessagesResult, ListAssistantThreadsArgs,
    ListAssistantThreadsResult, ListChatBindingsForJobArgs, ListChatBindingsForJobResult,
    ListJobFilesArgs, ListJobFilesResult, ListJobMessagesArgs, ListJobMessagesResult, ListJobsArgs,
    ListJobsResult, ListPersonasArgs, ListPersonasResult, ListProposedPatchesArgs,
    ListProposedPatchesResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    ListScheduledPausePointsArgs, ListScheduledPausePointsResult, ListStagesArgs, ListStagesResult,
    OverridePreCheckAndResumeArgs, PauseJobArgs, PostJobMessageArgs, ProposedPatchListEntry,
    ReadJobFileArgs, ReadJobFileResult, RejectScopePatchArgs, RemoveRepoArgs, RerunJobArgs,
    ResetJobArgs, ResumeJobArgs, RevertScopePatchArgs, RevertScopePatchResult, RunnerInfo,
    ScopePatchActionResult, ScopePatchResolution, ServerFeatureFlags, ServerInfo,
    SetAssistantThreadModeArgs, SetAssistantThreadModeResult, SetJobPolicyArgs, StageRollup,
    StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs, SubmitJobArgs,
    UpdateChatMessageDeliveryArgs, UpdateJobArgs, UpdateJobScopeArgs, UpdateJobScopeResult,
    UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadAssistantAttachmentArgs,
    UploadAssistantAttachmentResult, UploadChatAttachmentArgs, UploadChatAttachmentResult,
    UpsertPersonaArgs, UserPromptSnippet, WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs,
    WriteJobFileResult,
};
pub use server::RpcServer;
pub use subscribe::{EventFilter, EventStream, Since};
