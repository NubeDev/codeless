//! Supervisor tool surface (JOB-CHAT.md (C2) §Tool surface).
//!
//! The supervisor's read tools route through `SqliteStore` reads and
//! `EventBus` event-table reads — the same write-side helpers the
//! existing RPCs already use. The single write tool, `post_chat_message`,
//! inserts a row into `chat_messages` and publishes the matching
//! `ChatMessageAppended` envelope, mirroring the contract pinned by
//! `rpc::job_chat::post_job_message`. Holding the supervisor's voice
//! to this one append-and-publish path is what makes the chat thread
//! the supervisor's only side channel (R3 of JOB-CHAT.md "Hard rules").
//!
//! Why this lives next to the supervisor module rather than under
//! `rpc/`: every method is callable in-process without crossing the
//! RPC trait, so the supervisor (which itself never imports the
//! `RpcServer` trait — it owns the runtime, it does not consume it)
//! gets a typed surface without round-tripping through serde. Tests
//! that want the same behaviour go through the public RPC instead.
//!
//! Filesystem reads (`read_handover`, `read_stage_log`, `read_notes`)
//! resolve against a `worktree_root` supplied by the caller. The
//! supervisor is constructed with `None` in tests that do not need the
//! on-disk reads; production wiring threads the same root the
//! `WorktreeManager` uses for the Run's worktree so the path layout
//! matches the rest of the runtime.

pub mod actions;
pub use actions::{AdHocOutcome, AD_HOC_PREVIEW_WINDOW};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use codeless_types::{
    ChatMessage, ChatRole, ChatTransport, Event, EventEnvelope, JobId, JobStatus, MessageId, Stage,
    StageId, StageStatus, UnixMillis,
};

use crate::event_bus::EventBus;
use crate::handover::handover_path;
use crate::store::{InsertChatMessage, SqliteStore};
use crate::time::now_ms;

/// Upper bound on `read_events.limit`. Mirrors `LIST_LIMIT_MAX` on the
/// chat-history RPC: large enough for "summarise the last ~5 minutes"
/// answers, small enough that a confused supervisor cannot pull the
/// full events table on one tool call.
pub const READ_EVENTS_LIMIT_MAX: u32 = 500;

/// One row read by `read_notes`. The supervisor surface is text-only;
/// binary attachments are not surfaced here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFile {
    pub filename: String,
    pub body: String,
}

/// Snapshot the supervisor's `get_job_state` tool returns. Narrower
/// than the full `Job` + `Stage` rows: the supervisor only needs
/// enough to answer "what stage is it on?" and "how long has it been
/// running?" without re-reading the DB for follow-ups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStateView {
    pub job_id: JobId,
    pub status: JobStatus,
    pub started_at: Option<UnixMillis>,
    pub current_stage: Option<StageSummary>,
    pub stage_count: u32,
}

/// Compact projection of one `Stage` row for the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSummary {
    pub id: StageId,
    pub ordinal: u32,
    pub name: String,
    pub status: StageStatus,
    pub started_at: Option<UnixMillis>,
}

impl From<&Stage> for StageSummary {
    fn from(s: &Stage) -> Self {
        Self {
            id: s.id,
            ordinal: s.ordinal,
            name: s.name.clone(),
            status: s.status,
            started_at: s.started_at,
        }
    }
}

/// Bundled supervisor tool surface. Construct one per Run; the same
/// instance is reused across the supervisor's reactor loop.
pub struct Tools {
    bus: Arc<EventBus>,
    store: Arc<SqliteStore>,
    worktree_root: Option<PathBuf>,
}

impl Tools {
    pub fn new(bus: Arc<EventBus>, store: Arc<SqliteStore>) -> Self {
        Self {
            bus,
            store,
            worktree_root: None,
        }
    }

    /// Filesystem root for `read_handover`, `read_stage_log`,
    /// `read_notes`. `None` (the default) makes those tools return
    /// `NotConfigured` so tests that only exercise the DB surface do
    /// not have to provision a temp directory.
    pub fn with_worktree_root(mut self, root: PathBuf) -> Self {
        self.worktree_root = Some(root);
        self
    }

    /// Shared handle on the event bus. Used by the reactor in
    /// `supervisor::mod` to open its `subscribe_since` stream without
    /// owning a second `Arc<EventBus>` field alongside the tools.
    pub(crate) fn bus_arc(&self) -> Arc<EventBus> {
        Arc::clone(&self.bus)
    }

    /// Summarise the Job's current state for chat replies. The
    /// `current_stage` resolution rule is "the highest-ordinal stage
    /// whose status is `Running`, falling back to the highest-ordinal
    /// stage at all if none is running" — the same projection a human
    /// reading the stages list would make.
    pub async fn get_job_state(&self, job_id: JobId) -> Result<JobStateView, ToolError> {
        let job = self
            .store
            .get_job(job_id)
            .await
            .map_err(ToolError::Db)?
            .ok_or(ToolError::NotFound)?;
        let stages = self
            .store
            .list_stages_for_job(job_id)
            .await
            .map_err(ToolError::Db)?;
        let current = stages
            .iter()
            .filter(|s| s.stage.status == StageStatus::Running)
            .max_by_key(|s| s.stage.ordinal)
            .or_else(|| stages.iter().max_by_key(|s| s.stage.ordinal))
            .map(|s| StageSummary::from(&s.stage));
        Ok(JobStateView {
            job_id,
            status: job.status,
            started_at: job.started_at,
            current_stage: current,
            stage_count: stages.len() as u32,
        })
    }

    /// Shared handle on the store. Used by the supervisor's terminal
    /// summary path to read the per-stage `failure_detail` column
    /// without re-issuing the `get_job_state` projection (which drops
    /// the detail field).
    pub(crate) fn store_arc(&self) -> Arc<SqliteStore> {
        Arc::clone(&self.store)
    }

    /// Read the most recent `limit` persisted events for this Job in
    /// cursor-ascending order. `limit` is clamped to
    /// [`READ_EVENTS_LIMIT_MAX`].
    pub async fn read_events(
        &self,
        job_id: JobId,
        limit: u32,
    ) -> Result<Vec<EventEnvelope>, ToolError> {
        let limit = limit.clamp(1, READ_EVENTS_LIMIT_MAX);
        self.bus
            .fetch_recent_for_job(job_id, limit)
            .await
            .map_err(ToolError::Db)
    }

    /// Read `runs/<job_id>/<stage_id>/handover.md` from the worktree
    /// root. Returns `NotFound` when the file is missing — a stage
    /// that has not yet produced a handover is a normal state, not an
    /// error.
    pub async fn read_handover(
        &self,
        job_id: JobId,
        stage_id: StageId,
    ) -> Result<String, ToolError> {
        let root = self
            .worktree_root
            .as_deref()
            .ok_or(ToolError::NotConfigured)?;
        let path = handover_path(root, job_id, stage_id);
        read_text_file(&path).await
    }

    /// Return the job row's `template_yaml`. `None` when the row was
    /// submitted prompt-only (no template). The supervisor uses this
    /// to answer "what stages are left?" without re-parsing the YAML
    /// out of the on-disk file.
    pub async fn read_template(&self, job_id: JobId) -> Result<Option<String>, ToolError> {
        let job = self
            .store
            .get_job(job_id)
            .await
            .map_err(ToolError::Db)?
            .ok_or(ToolError::NotFound)?;
        Ok(job.template_yaml)
    }

    /// Read `runs/<job_id>/<stage_id>/log.md` — the per-stage activity
    /// log the recorder writes alongside the handover.
    pub async fn read_stage_log(
        &self,
        job_id: JobId,
        stage_id: StageId,
    ) -> Result<String, ToolError> {
        let root = self
            .worktree_root
            .as_deref()
            .ok_or(ToolError::NotConfigured)?;
        let path = root
            .join("runs")
            .join(job_id.to_string())
            .join(stage_id.to_string())
            .join("log.md");
        read_text_file(&path).await
    }

    /// Read every file under `runs/<job_id>/notes/`, sorted by
    /// filename so the supervisor sees notes in roughly chronological
    /// order (the `<ts>-…` filename convention from JOB-CHAT.md's
    /// `add_job_note` tool). Missing directory → empty `Vec`.
    pub async fn read_notes(&self, job_id: JobId) -> Result<Vec<NoteFile>, ToolError> {
        let root = self
            .worktree_root
            .as_deref()
            .ok_or(ToolError::NotConfigured)?;
        let dir = root.join("runs").join(job_id.to_string()).join("notes");
        let read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ToolError::Io(e)),
        };
        let mut entries = collect_files(read_dir).await?;
        entries.sort();
        let mut out = Vec::with_capacity(entries.len());
        for path in entries {
            let body = read_text_file(&path).await?;
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            out.push(NoteFile { filename, body });
        }
        Ok(out)
    }

    /// The supervisor's only write tool. Inserts an assistant-role
    /// row with `transport=Supervisor` and publishes the matching
    /// `ChatMessageAppended` envelope. Mirrors `post_job_message` on
    /// the RPC surface — same shape, no `external_id` (supervisor
    /// rows are uniquified by their `id` PK) and no metadata.
    pub async fn post_chat_message(
        &self,
        job_id: JobId,
        body: String,
    ) -> Result<ChatMessage, ToolError> {
        if self
            .store
            .get_job(job_id)
            .await
            .map_err(ToolError::Db)?
            .is_none()
        {
            return Err(ToolError::NotFound);
        }
        let now = now_ms();
        let msg = ChatMessage {
            id: MessageId::new(),
            job_id,
            run_id: None,
            transport: ChatTransport::Supervisor,
            external_id: None,
            thread_key: None,
            author: "supervisor".to_string(),
            role: ChatRole::Assistant,
            body,
            metadata_json: None,
            created_at: now,
        };
        match self
            .store
            .insert_chat_message(&msg)
            .await
            .map_err(ToolError::Db)?
        {
            InsertChatMessage::Inserted => {}
            // The supervisor's own messages are uniquified by the
            // ULID PK alone (no `external_id`), so a duplicate here
            // is a bug — surface it rather than silently swallow.
            InsertChatMessage::DuplicateExternalId => return Err(ToolError::DuplicateInsert),
        }
        let event = Event::ChatMessageAppended {
            job_id,
            message: msg.clone(),
        };
        self.bus
            .publish(Some(job_id), None, None, event, now)
            .await
            .map_err(ToolError::Db)?;
        Ok(msg)
    }
}

/// Typed errors for the tool surface. Distinct from `RpcError` so the
/// reactor inside `mod.rs` can match on `NotFound` vs `Db` without
/// sniffing strings; the reactor's chat reply pathway converts a
/// `Db`/`Io` failure into a one-line "I could not read that" chat
/// message rather than crashing the supervisor task.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("not found")]
    NotFound,
    #[error("tool not configured (no worktree root)")]
    NotConfigured,
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("duplicate insert on supervisor message")]
    DuplicateInsert,
}

async fn read_text_file(path: &Path) -> Result<String, ToolError> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ToolError::NotFound),
        Err(e) => Err(ToolError::Io(e)),
    }
}

async fn collect_files(mut read_dir: tokio::fs::ReadDir) -> Result<Vec<PathBuf>, ToolError> {
    let mut out = Vec::new();
    while let Some(entry) = read_dir.next_entry().await.map_err(ToolError::Io)? {
        let path = entry.path();
        if path.is_file() {
            out.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
    use codeless_types::{GitAuth, Stage, StageId};

    async fn fresh_rpc_with_job() -> (Arc<InProcessRpc>, JobId) {
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = rpc
            .add_repo(AddRepoArgs {
                name: "r".into(),
                clone_url: "u".into(),
                default_branch: "main".into(),
                local_path: "/tmp".into(),
                git_auth: GitAuth::Token {
                    env_var: "X".into(),
                },
                concurrency_cap: None,
                default_runner: None,
            })
            .await
            .unwrap();
        let job = rpc
            .submit_job(SubmitJobArgs {
                repo_id: repo.id,
                prompt: Some("p".into()),
                template_yaml: None,
                runner: "mock".into(),
                branch: "b".into(),
                workspace_mode: None,
                cost_cap_cents: 0,
                wall_clock_cap_ms: 0,
                model: None,
                permission_mode: None,
                effort: None,
                system_prompt: None,
                persona_id: None,
                auto_bypass_policy: None,
                start_immediately: false,
            })
            .await
            .unwrap();
        (Arc::new(rpc), job.id)
    }

    fn stage_running(job_id: JobId, ordinal: u32, name: &str) -> Stage {
        Stage {
            id: StageId::new(),
            job_id,
            ordinal,
            name: name.into(),
            status: StageStatus::Running,
            verify_cmd: None,
            started_at: Some(now_ms()),
            ended_at: None,
            session_id: None,
            goal: None,
            acceptance: None,
            last_activity_at: None,
            archived: false,
            persona_id: None,
            bypassed_at: None,
            bypassed_reason: None,
            failure_class: None,
            failure_detail: None,
        }
    }

    #[tokio::test]
    async fn get_job_state_picks_the_running_stage() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let stage = stage_running(job_id, 2, "stage 2: do the thing");
        rpc.store().insert_stage(&stage).await.unwrap();
        let tools = Tools::new(rpc.bus().clone(), rpc.store().clone());
        let view = tools.get_job_state(job_id).await.unwrap();
        let cur = view.current_stage.expect("must surface the running stage");
        assert_eq!(cur.ordinal, 2);
        assert_eq!(cur.name, "stage 2: do the thing");
        assert_eq!(view.stage_count, 1);
    }

    #[tokio::test]
    async fn post_chat_message_inserts_assistant_supervisor_row() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let tools = Tools::new(rpc.bus().clone(), rpc.store().clone());
        let msg = tools
            .post_chat_message(job_id, "hello from the supervisor".into())
            .await
            .unwrap();
        assert_eq!(msg.transport, ChatTransport::Supervisor);
        assert_eq!(msg.role, ChatRole::Assistant);
        assert_eq!(msg.author, "supervisor");
        let listed = rpc
            .store()
            .list_chat_messages(job_id, None, 10)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, msg.id);
    }
}
