use serde::{Deserialize, Serialize};

use crate::id::{TaskId, TodoId};
use crate::time::UnixMillis;

/// Todo lifecycle. `Skipped` is a terminal "did not run, will not run"
/// state distinct from `Failed` — used for the `git` trio item when a
/// stage produced no diff, and for any planner-injected item the runner
/// resolved away (e.g. an investigation that proved unnecessary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
    Failed,
}

/// Origin of a todo. The closing trio (`Checks`, `Docs`, `Git`) is
/// runtime-injected so a misbehaving runner cannot silently skip it;
/// `Runner` items come from the runner's own plan tool (`TodoWrite`
/// for Claude Code, equivalent for Codex). `Planner` items are reserved
/// for the future planner-driven path where the stage's checklist is
/// declared up-front rather than discovered as the runner works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum TodoKind {
    Runner,
    Planner,
    Checks,
    Docs,
    Git,
}

/// One user-visible sub-step inside a task — see `DOCS/SCOPE.md` Todo
/// row and `DOCS/JOB-UI.md` "Todo rows (nested under a tick)". Display
/// is the contract; the truth of "stage done" is still the verify gate
/// plus the push, not the checklist. The closing trio (`Checks`,
/// `Docs`, `Git`) is the exception: the runtime refuses to fire
/// `StageCompleted` until all three trio todos are `Done` or `Skipped`,
/// so the UI's green checkmarks on those three rows are load-bearing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Todo {
    pub id: TodoId,
    pub task_id: TaskId,
    /// Position within the task's checklist. Trio items sort last by
    /// convention (`Checks` < `Docs` < `Git`) — the recorder writes
    /// them with the three highest ordinals it has seen for the task.
    pub ordinal: u32,
    pub title: String,
    pub status: TodoStatus,
    pub kind: TodoKind,
    pub created_at: UnixMillis,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    /// Human-readable reason this todo ended `Failed`. Always `None`
    /// for `Done`, `Skipped`, or non-terminal rows. Populated by the
    /// trio emitters (handover write error, verify-step exit code, git
    /// commit error) and used by the closing-trio gate to build the
    /// stage's `failure_detail` so the UI shows *which* rail failed
    /// and *why*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

impl TodoKind {
    /// The three closing-trio kinds, in display order.
    pub const TRIO: [TodoKind; 3] = [TodoKind::Checks, TodoKind::Docs, TodoKind::Git];

    pub fn is_trio(self) -> bool {
        matches!(self, TodoKind::Checks | TodoKind::Docs | TodoKind::Git)
    }
}

impl TodoStatus {
    /// `Done` or `Skipped`. Used by the stage-completion gate to test
    /// whether the trio is satisfied without distinguishing "ran and
    /// passed" from "intentionally skipped" — both unblock the stage.
    pub fn is_resolved(self) -> bool {
        matches!(self, TodoStatus::Done | TodoStatus::Skipped)
    }
}
