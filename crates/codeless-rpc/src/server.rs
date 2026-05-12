use async_trait::async_trait;
use codeless_types::{Job, Repo, Review};

use crate::error::RpcResult;
use crate::methods::{
    AddRepoArgs, ApproveReviewArgs, CommentReviewArgs, FsCwdResult, FsReadDirArgs, FsReadDirResult,
    FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GetJobArgs,
    ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    RemoveRepoArgs, StopJobArgs, StopReviewArgs, SubmitJobArgs,
};
use crate::subscribe::{EventFilter, EventStream, Since};

/// The single typed entry point every transport adapts. Browser SSE/REST,
/// Tauri IPC, and the CLI's in-process call site all reach the runtime
/// through this trait — see SCOPE.md "Rule 1 — One transport interface,
/// many implementations".
///
/// Why the entire surface lives on one trait, instead of splitting per
/// resource: it makes the wire schema enumerable. Phase 3 walks the
/// methods, generates HTTP routes and a `specta` TS interface, and the
/// browser side is shaped automatically. Splitting the trait would
/// force the same enumeration to live in a separate registry.
///
/// `async_trait` is used (rather than native `async fn` in traits) so
/// the trait remains object-safe for `Arc<dyn RpcServer>` storage in
/// transport adapters.
#[async_trait]
pub trait RpcServer: Send + Sync + 'static {
    async fn add_repo(&self, args: AddRepoArgs) -> RpcResult<Repo>;
    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()>;
    async fn list_repos(&self) -> RpcResult<ListReposResult>;

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job>;
    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job>;
    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult>;
    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()>;

    async fn list_reviews(&self, args: ListReviewsArgs) -> RpcResult<ListReviewsResult>;
    /// Resolve a `Pending` review to `Approved`. Rejects with
    /// `Conflict` if the review has already been resolved; rejects
    /// with `NotFound` for an unknown id. Publishes `review-approved`
    /// on success.
    async fn approve_review(&self, args: ApproveReviewArgs) -> RpcResult<Review>;
    /// Attach a comment to a review. Only the comment field changes
    /// — the status stays put, even for already-resolved reviews, so
    /// post-mortem notes remain possible. Publishes `review-commented`.
    async fn comment_review(&self, args: CommentReviewArgs) -> RpcResult<Review>;
    /// Resolve a `Pending` review to `Stopped`. Same conflict / not-
    /// found semantics as `approve_review`. Publishes `review-stopped`.
    async fn stop_review(&self, args: StopReviewArgs) -> RpcResult<Review>;

    /// Streaming subscription. The returned stream replays events
    /// strictly after `since` (if `Some`) and then continues live.
    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream>;

    /// List one directory's immediate children. The path is relative
    /// to the server root; traversal outside the root is rejected by
    /// the host adapter, not at the wire level.
    async fn fs_read_dir(&self, args: FsReadDirArgs) -> RpcResult<FsReadDirResult>;

    /// Read a utf-8 text file. Binary and over-limit handling are not
    /// yet wired; non-utf-8 content surfaces as `InvalidArgument`.
    async fn fs_read_file(&self, args: FsReadFileArgs) -> RpcResult<FsReadFileResult>;

    /// Write a utf-8 text file. Parent directories must already exist
    /// — `fs_write_file` is for editor saves on known paths, not for
    /// scaffolding new project layouts (that surface arrives with the
    /// explorer's "new file" affordance and gets its own method).
    async fn fs_write_file(&self, args: FsWriteFileArgs) -> RpcResult<()>;

    /// Stat a single path. Missing paths return `kind: None` rather
    /// than `NotFound` so callers can probe existence without catching
    /// errors.
    async fn fs_stat(&self, args: FsStatArgs) -> RpcResult<FsStatResult>;

    /// Report the absolute server root the `fs_*` methods are scoped
    /// under. Returns `Internal` when no filesystem adapter is
    /// configured — same shape as the other `fs_*` methods when the
    /// runtime was built without `with_fs`.
    async fn fs_cwd(&self) -> RpcResult<FsCwdResult>;
}
