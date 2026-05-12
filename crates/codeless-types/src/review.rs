use serde::{Deserialize, Serialize};

use crate::id::{ReviewId, StageId};
use crate::time::UnixMillis;

/// Review-gate lifecycle. Reviews are a *state* on a Stage, not a node
/// in the tree (SCOPE.md "Tasks are the atomic re-runnable unit. Stages
/// are the verify-gated unit. Reviews are gates, not nodes…").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
    Stopped,
    RerunRequested,
}

/// A review row attached to a stage — see SCOPE.md Appendix A `reviews`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Review {
    pub id: ReviewId,
    pub stage_id: StageId,
    pub status: ReviewStatus,
    pub comment: Option<String>,
    pub requested_at: UnixMillis,
    pub resolved_at: Option<UnixMillis>,
}
