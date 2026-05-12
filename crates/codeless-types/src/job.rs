use serde::{Deserialize, Serialize};

use crate::id::{JobId, RepoId};
use crate::money::CostCents;
use crate::time::UnixMillis;

/// Lifecycle states for a job row. String form matches the
/// `jobs.status` column wire labels in SCOPE.md Appendix A.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    Queued,
    Running,
    AwaitingReview,
    Completed,
    Failed,
    Stopped,
}

/// Why a job left the running set early. `None` while running or after a
/// clean completion; populated when status is `Stopped` or `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    User,
    CostCap,
    WallClock,
    RunnerCrash,
}

/// One unit of work the user kicked off — see SCOPE.md Appendix A `jobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Job {
    pub id: JobId,
    pub repo_id: RepoId,
    pub status: JobStatus,
    pub stop_reason: Option<StopReason>,
    pub template_yaml: Option<String>,
    pub prompt: Option<String>,
    /// Runner kind chosen at submit time (e.g. `"claude"`, `"anthropic"`).
    pub runner: String,
    pub branch: String,
    /// `None` until the worktree has been provisioned. Preserved across
    /// crashes so a reaper can clean up after a dead leaseholder.
    pub worktree_path: Option<String>,
    pub cost_cap_cents: CostCents,
    pub wall_clock_cap_ms: i64,
    pub cost_cents: CostCents,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}
