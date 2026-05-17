//! Pure data types shared across the wire. See `DOCS/SCOPE.md` —
//! "Crate layout" — for why this crate has no I/O and depends only on
//! `serde` + `ulid`: the mobile shell depends on it transitively via
//! `codeless-client`, and adding host-only deps (tokio, std::process,
//! sqlx, ...) here would break that path.

pub mod allowed_tools;
pub mod assistant;
pub mod auto_bypass;
pub mod event;
pub mod fs;
pub mod git_auth;
pub mod handover;
pub mod id;
pub mod job;
pub mod money;
pub mod persona;
pub mod repo;
pub mod review;
pub mod review_gate;
pub mod scope_patch;
pub mod stage;
pub mod task;
pub mod time;
pub mod workspace;

pub use assistant::{
    AssistantAction, AssistantActionCard, AssistantActionStatus, AssistantAttachment,
    AssistantAttachmentCard, AssistantAttachmentCardItem, AssistantMessage, AssistantMessageRole,
    AssistantThread, AttachmentRef,
};
pub use auto_bypass::AutoBypassPolicy;
pub use event::{Event, EventCursor, EventEnvelope};
pub use fs::{FsEntry, FsEntryKind};
pub use git_auth::GitAuth;
pub use handover::{Handover, HandoverParseError};
pub use id::{
    AssistantAttachmentId, AssistantMessageId, AssistantThreadId, JobId, RepoId, ReviewId, StageId,
    TaskId,
};
pub use job::{Job, JobStatus, StopReason, WorkspaceMode};
pub use money::CostCents;
pub use persona::Persona;
pub use repo::Repo;
pub use review::{Review, ReviewStatus};
pub use review_gate::{PreCheckOutcome, ReviewVerdict};
pub use scope_patch::{
    ProposedScopePatch, ScopePatch, ScopePatchId, ScopePatchKind, ScopePatchTarget,
};
pub use stage::{FailureClass, Stage, StageStatus};
pub use task::{Task, TaskStatus};
pub use time::UnixMillis;
pub use workspace::{
    AttachWorkspaceArgs, AttachWorkspaceResult, AttachedWorkspace, DetachPolicy,
    DetachWorkspaceArgs, ListWorkspacesResult, ValidateWorkspacePathArgs,
    ValidateWorkspacePathResult, WorkspaceError, WorkspaceProblem,
};
