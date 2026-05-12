//! Pure data types shared across the wire. See `DOCS/SCOPE.md` —
//! "Crate layout" — for why this crate has no I/O and depends only on
//! `serde` + `ulid`: the mobile shell depends on it transitively via
//! `codeless-client`, and adding host-only deps (tokio, std::process,
//! sqlx, ...) here would break that path.

pub mod event;
pub mod fs;
pub mod git_auth;
pub mod handover;
pub mod id;
pub mod job;
pub mod money;
pub mod repo;
pub mod review;
pub mod stage;
pub mod task;
pub mod time;

pub use event::{Event, EventCursor, EventEnvelope};
pub use fs::{FsEntry, FsEntryKind};
pub use git_auth::GitAuth;
pub use handover::{Handover, HandoverParseError};
pub use id::{JobId, RepoId, ReviewId, StageId, TaskId};
pub use job::{Job, JobStatus, StopReason};
pub use money::CostCents;
pub use repo::Repo;
pub use review::{Review, ReviewStatus};
pub use stage::{Stage, StageStatus};
pub use task::{Task, TaskStatus};
pub use time::UnixMillis;
