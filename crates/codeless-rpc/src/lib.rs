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
    AttachWorkspaceArgs, AttachWorkspaceResult, AttachedWorkspace, DetachPolicy,
    DetachWorkspaceArgs, ListWorkspacesResult, Persona, ValidateWorkspacePathArgs,
    ValidateWorkspacePathResult, WorkspaceError, WorkspaceProblem,
};
pub use error::{RpcError, RpcResult};
pub use methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, CancelAssistantActionArgs,
    CancelAssistantActionResult, CancelChatTaskArgs, ChatAttachmentRef, ChatContext, ChatMode,
    ClaudeStatus, CommentReviewArgs, ConfirmAssistantActionArgs, ConfirmAssistantActionResult,
    CreateAssistantThreadArgs, DeleteAssistantThreadArgs, DeleteJobArgs, DeleteJobFileArgs,
    DeletePersonaArgs, DraftJobFromConversationArgs, FsCreateDirArgs, FsCreateFileArgs,
    FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs, FsReadDirResult, FsReadFileArgs,
    FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreeEntry, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, GetPersonaArgs, JobContextRef, JobDiffArgs, JobDiffFile,
    JobDiffResult, JobFileEntry, JobReportArgs, JobReportEventTally, JobReportResult,
    JobReportSpecChange, JobReportStage, JobReportToolCall, JobReportTurn,
    ListAssistantMessagesArgs, ListAssistantMessagesResult, ListAssistantThreadsArgs,
    ListAssistantThreadsResult, ListJobFilesArgs, ListJobFilesResult, ListJobsArgs, ListJobsResult,
    ListPersonasArgs, ListPersonasResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    ListStagesArgs, ListStagesResult, PauseJobArgs, ReadJobFileArgs, ReadJobFileResult,
    RemoveRepoArgs, RerunJobArgs, ResetJobArgs, ResumeJobArgs, RunnerInfo, ServerInfo, StageRollup,
    StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs, SubmitJobArgs,
    UpdateJobArgs, UpdateJobScopeArgs, UpdateJobScopeResult, UpdateJobTemplateArgs,
    UpdateJobTemplateResult, UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult,
    UploadChatAttachmentArgs, UploadChatAttachmentResult, UpsertPersonaArgs, UserPromptSnippet,
    WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs, WriteJobFileResult,
};
pub use server::RpcServer;
pub use subscribe::{EventFilter, EventStream, Since};
