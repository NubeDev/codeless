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

pub use error::{RpcError, RpcResult};
pub use methods::{
    AddRepoArgs, ApproveReviewArgs, ClaudeStatus, CommentReviewArgs, DeleteJobFileArgs,
    FsCwdResult, FsReadDirArgs, FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs,
    FsStatResult, FsWriteFileArgs, GcWorktreeEntry, GcWorktreesArgs, GcWorktreesResult, GetJobArgs,
    JobDiffArgs, JobDiffFile, JobDiffResult, JobFileEntry, ListJobFilesArgs, ListJobFilesResult,
    ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    ReadJobFileArgs, ReadJobFileResult, RemoveRepoArgs, RerunJobArgs, RunnerInfo, ServerInfo,
    StopJobArgs, StopReviewArgs, SubmitJobArgs, UpdateJobTemplateArgs, UpdateJobTemplateResult,
    WriteJobFileArgs, WriteJobFileResult,
};
pub use server::RpcServer;
pub use subscribe::{EventFilter, EventStream, Since};
