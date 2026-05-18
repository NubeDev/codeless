//! Persistence for pre-armed supervisor goals (JOB-CHAT.md §C3).
//!
//! Every row in `supervisor_goals` is a user "if X then Y" intent the
//! supervisor recorded. The supervisor lifecycle in
//! `crate::supervisor` rehydrates open rows on boot — without that, a
//! process restart silently loses the user's authorisation and
//! JOB-CHAT.md's "if it runs >1h, stop it" example breaks. The
//! invariants the table relies on (closed kind set, typed
//! `condition_json` / `action_json`, monotonic status walk) are
//! enforced here at write time so the supervisor reactor can treat
//! anything it reads back as already-valid.
//!
//! Validation strategy: rather than push enum validation into the
//! SQL layer (sqlite has no native enum), the insert helper
//! re-serialises the caller-typed `GoalCondition` / `GoalAction`
//! into the persisted strings. A caller that constructed a goal
//! through this module cannot write an unrecognised `kind`, an
//! out-of-shape condition payload, or a mismatched
//! `kind`/`condition`/`action` triple — `Goal::new` and
//! `insert_goal` are the only writers and both go through the same
//! validator.

use codeless_types::{JobId, MessageId, UnixMillis};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::codec::{parse_id, serde_err};
use super::SqliteStore;

/// Closed v0.1 set of supervisor goal kinds (JOB-CHAT.md §C3).
/// The wire spellings match the SQL column verbatim — adapters and
/// the supervisor reactor compare values as these kebab-case strings,
/// never as Rust identifiers. Adding a kind requires both a doc PR
/// amending JOB-CHAT.md §C3 and a fresh migration, by design: a
/// supervisor reading rows written by a newer process must reject
/// what it cannot execute rather than silently treat unknown kinds
/// as no-ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisorGoalKind {
    DeadlineStop,
    ThresholdStop,
    EventNotify,
}

impl SupervisorGoalKind {
    /// Wire label for the `kind` column. Pattern-matched (not derived
    /// from `serde`) so the column spelling lives in one place and a
    /// future rename surfaces as a compile error.
    pub fn as_str(self) -> &'static str {
        match self {
            SupervisorGoalKind::DeadlineStop => "deadline-stop",
            SupervisorGoalKind::ThresholdStop => "threshold-stop",
            SupervisorGoalKind::EventNotify => "event-notify",
        }
    }

    fn parse(s: &str) -> Result<Self, GoalValidationError> {
        match s {
            "deadline-stop" => Ok(SupervisorGoalKind::DeadlineStop),
            "threshold-stop" => Ok(SupervisorGoalKind::ThresholdStop),
            "event-notify" => Ok(SupervisorGoalKind::EventNotify),
            other => Err(GoalValidationError::UnknownKind(other.to_string())),
        }
    }
}

/// Numeric metric kinds the threshold-stop condition watches. v0.1
/// covers the two thresholds JOB-CHAT.md mentions ("if cost passes
/// $1, stop", "if wall-clock passes …"); more can land alongside a
/// new metric source on the supervisor reactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdMetric {
    CostCents,
    WallClockMs,
}

/// Typed condition payload. One variant per `SupervisorGoalKind`; the
/// `kind` <-> `condition` correspondence is enforced by
/// `Goal::validate` so a row whose kind is `deadline-stop` cannot
/// carry a threshold condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GoalCondition {
    /// Wall-clock deadline expressed in absolute milliseconds since
    /// the Unix epoch. Storing the absolute deadline (rather than a
    /// relative duration from goal creation) is what lets the
    /// supervisor re-arm the same `tokio::time::sleep_until` after a
    /// restart without re-anchoring to boot time.
    DeadlineStop { deadline_ms: i64 },
    /// Trip when `metric` crosses `threshold`. The comparison is
    /// "value >= threshold"; the supervisor reactor polls the metric
    /// and fires the goal on the first sample that crosses.
    ThresholdStop {
        metric: ThresholdMetric,
        threshold: i64,
    },
    /// Trip when a named Event variant lands on the bus. `event_kind`
    /// is the serde tag of the `Event` enum (e.g. `StageCompleted`);
    /// the supervisor's bus subscription filters by tag and fires on
    /// match.
    EventNotify { event_kind: String },
}

impl GoalCondition {
    fn kind(&self) -> SupervisorGoalKind {
        match self {
            GoalCondition::DeadlineStop { .. } => SupervisorGoalKind::DeadlineStop,
            GoalCondition::ThresholdStop { .. } => SupervisorGoalKind::ThresholdStop,
            GoalCondition::EventNotify { .. } => SupervisorGoalKind::EventNotify,
        }
    }
}

/// Typed action payload. The supervisor's `tools::actions` surface
/// is the executor — this enum is the persisted intent only. The
/// `PauseAfterStage` variant is parsed at write time but will produce
/// a structured `Failed` outcome when the supervisor reactor tries to
/// execute it: JOB-WORKFLOW (A.5) is the affordance that makes the
/// pause real, and it has not landed yet (see
/// `JOB-CHAT.md` §C3 — *`pause_after_stage` tool (no-op until
/// JOB-WORKFLOW (A.5))*). Keeping the variant in the enum lets the
/// store round-trip a parsed-but-inert intent so the chat audit
/// trail is honest about what the user asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GoalAction {
    /// Cancel the Run with `reason` (typically the body of the
    /// authorising user message). Same RPC path as the `[stop]`
    /// button in the run page; the resulting `JobStopped` envelope
    /// is byte-identical to the UI-triggered one.
    StopJob { reason: String },
    /// Append a `transport='supervisor'` row to `chat_messages` with
    /// the given body. The only non-destructive action variant.
    PostChatMessage { body: String },
    /// Pause the Run at the next stage boundary. Parsed today,
    /// executed later — `execution_state` returns
    /// `ExecutionState::NoOpFailed` so the eventual reactor wiring
    /// can short-circuit without re-parsing the action body.
    PauseAfterStage { stage_name: String },
}

impl GoalAction {
    /// Execution-readiness for the action. Today only
    /// `PauseAfterStage` returns `NoOpFailed`; the other two are
    /// `Ready` and are executed by `Tools::stop_job` /
    /// `Tools::post_chat_message`. The supervisor reactor reads this
    /// to decide whether to invoke the action or to mark the goal
    /// `fired` with a Failed audit reply.
    pub fn execution_state(&self) -> ExecutionState {
        match self {
            GoalAction::StopJob { .. } | GoalAction::PostChatMessage { .. } => {
                ExecutionState::Ready
            }
            GoalAction::PauseAfterStage { .. } => ExecutionState::NoOpFailed {
                reason: "pause_after_stage is a no-op until JOB-WORKFLOW (A.5) lands",
            },
        }
    }
}

/// Outcome of asking an action whether it is ready to execute. Kept
/// separate from `GoalStatus` because a `NoOpFailed` action still
/// transitions the goal to a terminal status — it just records that
/// no real side-effect happened — and the supervisor reactor needs
/// to distinguish "we fired the action and it ran" from "the action
/// is a parsed-but-inert placeholder".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Ready,
    NoOpFailed { reason: &'static str },
}

/// Status walk for a row. `armed` is the only non-terminal value;
/// the three terminal statuses are mutually exclusive and each has
/// its own `mark_*` helper so a typo on the wire cannot land an
/// unknown status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GoalStatus {
    Armed,
    Fired,
    Cancelled,
    Superseded,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Armed => "armed",
            GoalStatus::Fired => "fired",
            GoalStatus::Cancelled => "cancelled",
            GoalStatus::Superseded => "superseded",
        }
    }

    fn parse(s: &str) -> Result<Self, GoalValidationError> {
        match s {
            "armed" => Ok(GoalStatus::Armed),
            "fired" => Ok(GoalStatus::Fired),
            "cancelled" => Ok(GoalStatus::Cancelled),
            "superseded" => Ok(GoalStatus::Superseded),
            other => Err(GoalValidationError::UnknownStatus(other.to_string())),
        }
    }
}

/// Identity of one `supervisor_goals` row. A ULID minted by the
/// runtime on insert; the supervisor reactor uses it as the keying
/// term for in-memory timer arms so a `mark_*` call deterministically
/// dismisses the right arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SupervisorGoalId(pub ulid::Ulid);

impl SupervisorGoalId {
    pub fn new() -> Self {
        Self(ulid::Ulid::new())
    }
}

impl Default for SupervisorGoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SupervisorGoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for SupervisorGoalId {
    type Err = ulid::DecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ulid::Ulid::from_str(s).map(Self)
    }
}

/// One persisted supervisor goal. The `run_id` is currently typed as
/// `JobId` because JOB-WORKFLOW (B) (the Job/Run split that
/// introduces `RunId`) has not landed yet; the SQL column stays TEXT
/// and the value swaps from a Job-shaped ULID to a Run-shaped ULID
/// without a schema change. Authorisation is per-message:
/// `authorised_by` is the `chat_messages.id` of the user's "if X then
/// Y" turn, so the audit trail JOB-CHAT.md Hard rule 4 promises is a
/// foreign-key edge, not a free-text annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorGoal {
    pub id: SupervisorGoalId,
    pub run_id: JobId,
    pub condition: GoalCondition,
    pub action: GoalAction,
    pub authorised_by: MessageId,
    pub status: GoalStatus,
    pub created_at: UnixMillis,
    pub fired_at: Option<UnixMillis>,
}

impl SupervisorGoal {
    /// Convenience constructor that fills in the `armed` status and
    /// the `fired_at = None` invariant. The kind is derived from the
    /// condition variant — the table's `kind` column is redundant
    /// with the `condition_json` tag, kept as its own column so the
    /// rehydration scan can filter without parsing the JSON.
    pub fn new(
        run_id: JobId,
        condition: GoalCondition,
        action: GoalAction,
        authorised_by: MessageId,
        created_at: UnixMillis,
    ) -> Self {
        Self {
            id: SupervisorGoalId::new(),
            run_id,
            condition,
            action,
            authorised_by,
            status: GoalStatus::Armed,
            created_at,
            fired_at: None,
        }
    }

    pub fn kind(&self) -> SupervisorGoalKind {
        self.condition.kind()
    }
}

/// Reasons a goal can fail validation on write. Distinct variants so
/// the supervisor's error reply can quote the specific issue rather
/// than a generic "bad goal".
#[derive(Debug, thiserror::Error)]
pub enum GoalValidationError {
    #[error("unknown supervisor goal kind: {0}")]
    UnknownKind(String),
    #[error("unknown supervisor goal status: {0}")]
    UnknownStatus(String),
    #[error("malformed condition_json: {0}")]
    BadCondition(#[source] serde_json::Error),
    #[error("malformed action_json: {0}")]
    BadAction(#[source] serde_json::Error),
    #[error(
        "kind / condition mismatch: row kind = {row_kind}, condition shape = {condition_kind}"
    )]
    KindConditionMismatch {
        row_kind: &'static str,
        condition_kind: &'static str,
    },
}

/// Outcome of a `mark_*` transition. Most callers do not need to
/// know whether the row was already terminal; the supervisor reactor
/// uses this to log "I tried to cancel a goal that had already
/// fired" without treating that as an error case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOutcome {
    /// Row transitioned from `armed` to the requested terminal
    /// status.
    Transitioned,
    /// Row was already terminal (or missing); no SQL update landed.
    /// JOB-CHAT.md §C3 admits race cases — supervisor decides to fire
    /// while the user cancels in chat — so a no-op on the loser is
    /// expected behaviour, not an error.
    NoChange,
}

fn condition_kind_label(c: &GoalCondition) -> &'static str {
    match c {
        GoalCondition::DeadlineStop { .. } => "deadline-stop",
        GoalCondition::ThresholdStop { .. } => "threshold-stop",
        GoalCondition::EventNotify { .. } => "event-notify",
    }
}

fn validate(
    kind: SupervisorGoalKind,
    condition: &GoalCondition,
) -> Result<(), GoalValidationError> {
    if condition.kind() == kind {
        Ok(())
    } else {
        Err(GoalValidationError::KindConditionMismatch {
            row_kind: kind.as_str(),
            condition_kind: condition_kind_label(condition),
        })
    }
}

fn goal_from_row(row: SqliteRow) -> sqlx::Result<SupervisorGoal> {
    let id: String = row.try_get("id")?;
    let run_id: String = row.try_get("run_id")?;
    let kind_raw: String = row.try_get("kind")?;
    let condition_raw: String = row.try_get("condition_json")?;
    let action_raw: String = row.try_get("action_json")?;
    let authorised_by: String = row.try_get("authorised_by")?;
    let status_raw: String = row.try_get("status")?;
    let kind = SupervisorGoalKind::parse(&kind_raw)
        .map_err(|e| sqlx::Error::Decode(format!("supervisor_goals.kind: {e}").into()))?;
    let condition: GoalCondition = serde_json::from_str(&condition_raw).map_err(serde_err)?;
    validate(kind, &condition).map_err(|e| sqlx::Error::Decode(e.to_string().into()))?;
    let action: GoalAction = serde_json::from_str(&action_raw).map_err(serde_err)?;
    let status = GoalStatus::parse(&status_raw)
        .map_err(|e| sqlx::Error::Decode(format!("supervisor_goals.status: {e}").into()))?;
    Ok(SupervisorGoal {
        id: SupervisorGoalId(
            id.parse::<ulid::Ulid>()
                .map_err(|e| sqlx::Error::Decode(format!("supervisor_goals.id: {e}").into()))?,
        ),
        run_id: parse_id(&run_id)?,
        condition,
        action,
        authorised_by: parse_id(&authorised_by)?,
        status,
        created_at: UnixMillis(row.try_get("created_at")?),
        fired_at: row.try_get::<Option<i64>, _>("fired_at")?.map(UnixMillis),
    })
}

impl SqliteStore {
    /// Insert a new pre-armed goal. Validates the typed payloads (so
    /// a malformed condition / mismatched kind cannot land on disk)
    /// then writes the canonical JSON serialisation. The persisted
    /// `kind` column is derived from the condition variant — callers
    /// do not pass it separately — so the redundancy with
    /// `condition_json` stays consistent by construction.
    pub async fn insert_goal(&self, goal: &SupervisorGoal) -> Result<(), InsertGoalError> {
        let kind = goal.kind();
        validate(kind, &goal.condition).map_err(InsertGoalError::Validation)?;
        let condition_json =
            serde_json::to_string(&goal.condition).map_err(GoalValidationError::BadCondition)?;
        let action_json =
            serde_json::to_string(&goal.action).map_err(GoalValidationError::BadAction)?;
        sqlx::query(
            "INSERT INTO supervisor_goals \
             (id, run_id, kind, condition_json, action_json, authorised_by, \
              status, created_at, fired_at) \
             VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(goal.id.to_string())
        .bind(goal.run_id.to_string())
        .bind(kind.as_str())
        .bind(&condition_json)
        .bind(&action_json)
        .bind(goal.authorised_by.to_string())
        .bind(goal.status.as_str())
        .bind(goal.created_at.0)
        .bind(goal.fired_at.map(|t| t.0))
        .execute(&self.pool)
        .await
        .map_err(InsertGoalError::Sql)?;
        Ok(())
    }

    /// Rehydration query: every `armed` goal for the Run, ordered by
    /// `created_at` ascending so the supervisor re-arms timers in the
    /// same order the user authorised them. Backed by
    /// `idx_supervisor_goals_armed`; the partial index keeps the scan
    /// cost flat as terminal rows accumulate over the Run's lifetime.
    pub async fn list_armed_for_run(&self, run_id: JobId) -> sqlx::Result<Vec<SupervisorGoal>> {
        let rows = sqlx::query(
            "SELECT * FROM supervisor_goals \
             WHERE run_id = ? AND status = 'armed' \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(goal_from_row).collect()
    }

    /// Transition an `armed` row to `fired`. Idempotent: a second
    /// call with the same id (or a call against an already-terminal
    /// row) returns `NoChange` rather than erroring — the supervisor
    /// reactor races a deadline arm with a user cancel and the loser
    /// has to no-op cleanly.
    pub async fn mark_fired(
        &self,
        id: SupervisorGoalId,
        fired_at: UnixMillis,
    ) -> sqlx::Result<MarkOutcome> {
        self.mark_terminal(id, GoalStatus::Fired, Some(fired_at))
            .await
    }

    /// Transition an `armed` row to `cancelled`. The user typed "never
    /// mind, let it run" — the goal goes away but the row stays for
    /// the audit trail. `fired_at` is left NULL on this path because
    /// nothing fired.
    pub async fn mark_cancelled(&self, id: SupervisorGoalId) -> sqlx::Result<MarkOutcome> {
        self.mark_terminal(id, GoalStatus::Cancelled, None).await
    }

    /// Transition an `armed` row to `superseded`. The Run reached a
    /// terminal status before the condition could trip; the goal is
    /// no longer reachable and the supervisor exits without firing.
    /// `fired_at` stays NULL — the goal did not produce an action.
    pub async fn mark_superseded(&self, id: SupervisorGoalId) -> sqlx::Result<MarkOutcome> {
        self.mark_terminal(id, GoalStatus::Superseded, None).await
    }

    async fn mark_terminal(
        &self,
        id: SupervisorGoalId,
        status: GoalStatus,
        fired_at: Option<UnixMillis>,
    ) -> sqlx::Result<MarkOutcome> {
        debug_assert!(
            status != GoalStatus::Armed,
            "mark_terminal must move out of armed"
        );
        // The `status = 'armed'` predicate is what makes the helper
        // safe under concurrent callers: only one transition wins,
        // every other writer's UPDATE finds no row and returns
        // `NoChange`. SQLite's row locking serialises the writes,
        // so the final state is one of the four terminal outcomes
        // (never partial).
        let result = sqlx::query(
            "UPDATE supervisor_goals \
             SET status = ?, fired_at = ? \
             WHERE id = ? AND status = 'armed'",
        )
        .bind(status.as_str())
        .bind(fired_at.map(|t| t.0))
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(if result.rows_affected() == 0 {
            MarkOutcome::NoChange
        } else {
            MarkOutcome::Transitioned
        })
    }
}

/// Error returned by `insert_goal`. Carries the validator error
/// distinct from the underlying SQL error so the supervisor can
/// reply differently to "your `if X then Y` parsed wrong" versus
/// "the database is down".
#[derive(Debug, thiserror::Error)]
pub enum InsertGoalError {
    #[error(transparent)]
    Validation(GoalValidationError),
    #[error(transparent)]
    Sql(sqlx::Error),
}

impl From<GoalValidationError> for InsertGoalError {
    fn from(e: GoalValidationError) -> Self {
        InsertGoalError::Validation(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::MIGRATOR;
    use codeless_types::{
        ChatMessage, ChatRole, ChatTransport, CostCents, GitAuth, Job, JobStatus, Repo, RepoId,
        WorkspaceMode,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_store_with_message() -> (SqliteStore, JobId, MessageId) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool);
        let now = UnixMillis(0);
        let repo = Repo {
            id: RepoId::new(),
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: now,
            updated_at: now,
        };
        store.insert_repo(&repo).await.unwrap();
        let job = Job {
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: now,
        };
        store.insert_job(&job).await.unwrap();
        // The authorising chat row — supervisor_goals.authorised_by
        // is a FK on chat_messages.id, so the test seed has to insert
        // a real row.
        let msg = ChatMessage {
            id: MessageId::new(),
            job_id: job.id,
            run_id: None,
            transport: ChatTransport::Web,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: "if this runs more than an hour, stop it".into(),
            metadata_json: None,
            created_at: now,
        };
        store.insert_chat_message(&msg).await.unwrap();
        (store, job.id, msg.id)
    }

    fn deadline_goal(run_id: JobId, auth: MessageId) -> SupervisorGoal {
        SupervisorGoal::new(
            run_id,
            GoalCondition::DeadlineStop {
                deadline_ms: 3_600_000,
            },
            GoalAction::StopJob {
                reason: "ran past 1h budget".into(),
            },
            auth,
            UnixMillis(1),
        )
    }

    #[tokio::test]
    async fn insert_and_list_armed_roundtrips_typed_payloads() {
        let (store, run_id, auth) = fresh_store_with_message().await;
        let g = deadline_goal(run_id, auth);
        store.insert_goal(&g).await.unwrap();
        let armed = store.list_armed_for_run(run_id).await.unwrap();
        assert_eq!(armed.len(), 1);
        let got = &armed[0];
        assert_eq!(got.id, g.id);
        assert_eq!(got.kind(), SupervisorGoalKind::DeadlineStop);
        assert_eq!(got.condition, g.condition);
        assert_eq!(got.action, g.action);
        assert_eq!(got.authorised_by, auth);
        assert_eq!(got.status, GoalStatus::Armed);
        assert!(got.fired_at.is_none());
    }

    #[tokio::test]
    async fn insert_rejects_kind_condition_mismatch_in_decode() {
        // The validator catches the mismatch on the write path; the
        // direct path through `Goal::new` cannot construct a
        // mismatched goal because `kind()` is derived from the
        // condition. To exercise the validator we insert raw SQL and
        // confirm the read decode rejects the row.
        let (store, run_id, auth) = fresh_store_with_message().await;
        sqlx::query(
            "INSERT INTO supervisor_goals \
             (id, run_id, kind, condition_json, action_json, authorised_by, \
              status, created_at, fired_at) \
             VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(SupervisorGoalId::new().to_string())
        .bind(run_id.to_string())
        .bind("deadline-stop")
        .bind(r#"{"kind":"threshold-stop","metric":"cost_cents","threshold":100}"#)
        .bind(r#"{"kind":"stop-job","reason":"x"}"#)
        .bind(auth.to_string())
        .bind("armed")
        .bind(0_i64)
        .bind::<Option<i64>>(None)
        .execute(store.pool())
        .await
        .unwrap();
        let err = store.list_armed_for_run(run_id).await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("kind / condition mismatch"),
            "expected validation error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn mark_fired_then_repeat_returns_no_change() {
        let (store, run_id, auth) = fresh_store_with_message().await;
        let g = deadline_goal(run_id, auth);
        store.insert_goal(&g).await.unwrap();
        assert_eq!(
            store.mark_fired(g.id, UnixMillis(5)).await.unwrap(),
            MarkOutcome::Transitioned
        );
        // Second call: the row has already left `armed`, so the
        // UPDATE's WHERE clause matches nothing.
        assert_eq!(
            store.mark_fired(g.id, UnixMillis(6)).await.unwrap(),
            MarkOutcome::NoChange
        );
        // The rehydration scan must not return the row any more.
        assert!(store.list_armed_for_run(run_id).await.unwrap().is_empty());
        // Audit-trail invariant: the row still exists with its
        // terminal status and the `fired_at` from the first call.
        let row = sqlx::query("SELECT status, fired_at FROM supervisor_goals WHERE id = ?")
            .bind(g.id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
        let status: String = row.get("status");
        let fired_at: Option<i64> = row.get("fired_at");
        assert_eq!(status, "fired");
        assert_eq!(fired_at, Some(5));
    }

    #[tokio::test]
    async fn mark_cancelled_and_mark_superseded_are_terminal_no_op_on_repeat() {
        let (store, run_id, auth) = fresh_store_with_message().await;
        let g1 = deadline_goal(run_id, auth);
        let g2 = SupervisorGoal {
            id: SupervisorGoalId::new(),
            ..deadline_goal(run_id, auth)
        };
        store.insert_goal(&g1).await.unwrap();
        store.insert_goal(&g2).await.unwrap();
        assert_eq!(
            store.mark_cancelled(g1.id).await.unwrap(),
            MarkOutcome::Transitioned
        );
        assert_eq!(
            store.mark_superseded(g2.id).await.unwrap(),
            MarkOutcome::Transitioned
        );
        // Cross-status transitions all return NoChange — there is no
        // path back to `armed`.
        assert_eq!(
            store.mark_fired(g1.id, UnixMillis(9)).await.unwrap(),
            MarkOutcome::NoChange
        );
        assert_eq!(
            store.mark_cancelled(g2.id).await.unwrap(),
            MarkOutcome::NoChange
        );
        assert!(store.list_armed_for_run(run_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pause_after_stage_action_round_trips_and_reports_no_op_failed() {
        // Acceptance for "pause_after_stage is parsed but produces a
        // structured Failed (no-op until JOB-WORKFLOW (A.5))" — the
        // store accepts the action on the way in and surfaces the
        // NoOpFailed signal on the way out so the supervisor reactor
        // can short-circuit without invoking a runner-side pause.
        let (store, run_id, auth) = fresh_store_with_message().await;
        let g = SupervisorGoal::new(
            run_id,
            GoalCondition::EventNotify {
                event_kind: "StageCompleted".into(),
            },
            GoalAction::PauseAfterStage {
                stage_name: "verify".into(),
            },
            auth,
            UnixMillis(2),
        );
        store.insert_goal(&g).await.unwrap();
        let got = &store.list_armed_for_run(run_id).await.unwrap()[0];
        match got.action.execution_state() {
            ExecutionState::NoOpFailed { reason } => {
                assert!(reason.contains("JOB-WORKFLOW (A.5)"));
            }
            ExecutionState::Ready => panic!("pause_after_stage must report NoOpFailed today"),
        }
    }

    #[tokio::test]
    async fn list_armed_skips_terminal_and_orders_by_created_at() {
        let (store, run_id, auth) = fresh_store_with_message().await;
        let g1 = SupervisorGoal {
            created_at: UnixMillis(10),
            ..deadline_goal(run_id, auth)
        };
        let g2 = SupervisorGoal {
            id: SupervisorGoalId::new(),
            created_at: UnixMillis(20),
            ..deadline_goal(run_id, auth)
        };
        let g3 = SupervisorGoal {
            id: SupervisorGoalId::new(),
            created_at: UnixMillis(30),
            ..deadline_goal(run_id, auth)
        };
        for g in [&g1, &g2, &g3] {
            store.insert_goal(g).await.unwrap();
        }
        store.mark_cancelled(g2.id).await.unwrap();
        let armed = store.list_armed_for_run(run_id).await.unwrap();
        let ids: Vec<_> = armed.iter().map(|g| g.id).collect();
        assert_eq!(ids, vec![g1.id, g3.id]);
    }
}
