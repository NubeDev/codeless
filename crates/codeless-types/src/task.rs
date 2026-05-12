use serde::{Deserialize, Serialize};

use crate::id::{StageId, TaskId};
use crate::money::CostCents;
use crate::time::UnixMillis;

/// Task lifecycle. Matches `tasks.status` wire labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Enqueued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// The atomic re-runnable unit inside a stage — see SCOPE.md Appendix A
/// `tasks`. `depends_on` carries DAG edges from day one so the event
/// schema is forward-compatible with topological execution (SCOPE.md
/// Rule 4); linear-mode runtimes leave it empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Task {
    pub id: TaskId,
    pub stage_id: StageId,
    pub ordinal: u32,
    pub status: TaskStatus,
    pub depends_on: Vec<TaskId>,
    /// `"<pid>:<startup-nonce>"` while leased; `None` while idle. The
    /// startup-nonce is minted once per core-process start so PID-reuse
    /// after a crash can't be mistaken for an alive holder.
    pub lease_holder: Option<String>,
    pub lease_expires_at: Option<UnixMillis>,
    pub cost_cents: CostCents,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
}
