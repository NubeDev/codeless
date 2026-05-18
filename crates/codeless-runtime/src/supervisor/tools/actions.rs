//! Supervisor action tools (JOB-CHAT.md (C3) §Action tools).
//!
//! Two writable side-effects that the supervisor can produce on the
//! user's behalf: stop the Run, and append a note to the chat thread.
//! Each routes through the same `SqliteStore` + `EventBus` surfaces
//! that the equivalent UI button uses, so the resulting `JobStopped`
//! envelope is byte-identical to the one the run page's `[stop]`
//! button publishes and the resulting `ChatMessageAppended` row carries
//! the same shape every transport already renders. Hard rule 5 of
//! JOB-CHAT.md ("action-tool invocations emit events") is what binds
//! the supervisor surface to the rest of the system — the audit trail
//! the events table already maintains is the audit trail for chat-
//! driven actions too.
//!
//! Two regimes, per Hard rule 4:
//!
//! - **Pre-armed** (`stop_job`, `add_job_note`): the user already said
//!   "if X then Y" earlier in the thread and the supervisor recorded
//!   the intent as a `supervisor_goals` row. When the condition trips
//!   the action runs immediately, no preview. The audit trail is the
//!   original authorising message plus the post-action summary, both
//!   `chat_messages` rows.
//! - **Ad-hoc** (`stop_job_ad_hoc`): the supervisor reasoned its way
//!   to a destructive action and is *not* invoking a pre-armed goal.
//!   A 5-second preview row goes out first; if a user message starting
//!   with `wait` (case-insensitive, word-boundary terminated) lands
//!   inside the window the action stands down and the preview's
//!   `metadata.resolves` chain pairs the cancellation row to the
//!   preview row. Otherwise the action fires and the follow-up summary
//!   takes the same `resolves` slot.
//!
//! The events row for an action emitted from here carries no explicit
//! `actor` column (the `events` table predates the actor concept) —
//! the audit-trail proof that *the supervisor* triggered the stop
//! lives in `chat_messages`: every action emits a same-cursor
//! `ChatMessageAppended { transport: Supervisor }` envelope alongside
//! its `JobStopped` / `JobFileUpdated` partner, and the chat row's
//! `transport='supervisor'` is the per-action provenance the audit
//! reader needs.

use std::time::Duration;

use codeless_rpc::EventStream;
use codeless_types::{
    ChatMessage, ChatRole, ChatTransport, Event, JobId, JobStatus, MessageId, StopReason,
};
use futures_util::StreamExt;

use crate::event_bus::SubscribeFilter;
use crate::store::InsertChatMessage;
use crate::time::now_ms;

use super::{ToolError, Tools};

/// Default ad-hoc preview window. JOB-CHAT.md Hard rule 4 pins five
/// seconds as the time a user has to type "wait" before a supervisor-
/// decided destructive action fires. The test entry point
/// `stop_job_ad_hoc_with_window` lets the e2e suite shorten this so the
/// tests do not block on a real five-second sleep — production code
/// always uses the default.
pub const AD_HOC_PREVIEW_WINDOW: Duration = Duration::from_secs(5);

/// Outcome of an ad-hoc action. The caller (the supervisor reactor in
/// production, the e2e harness in tests) inspects this so a "we stood
/// down" outcome can be logged differently from a "we fired" outcome
/// without parsing the chat thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdHocOutcome {
    /// The preview window elapsed without a `wait` message; the action
    /// ran and a summary row followed.
    Fired,
    /// A user posted `/^wait\b/i` during the window; the action was
    /// abandoned and a cancellation row was posted.
    Aborted,
}

impl Tools {
    /// Pre-armed `stop_job`. Fires immediately, no preview, posts a
    /// one-line audit-trail summary referencing the supplied `reason`
    /// (typically the body of the authorising user message — the
    /// `supervisor_goals.authorised_by → chat_messages.id → body`
    /// chain). The same `JobStopped { reason: User }` envelope the UI
    /// `[stop]` button publishes.
    pub async fn stop_job(&self, job_id: JobId, reason: String) -> Result<(), ToolError> {
        self.stop_job_inner(job_id).await?;
        let summary = format!("Stopped the job: {reason}");
        post_supervisor_row(self, job_id, ChatRole::Assistant, summary, None, None).await?;
        Ok(())
    }

    /// Ad-hoc `stop_job` with the default five-second preview window.
    /// See module docs for the regime split. Returns `Fired` or
    /// `Aborted` so the caller can branch on the outcome without
    /// re-reading the chat thread.
    pub async fn stop_job_ad_hoc(
        &self,
        job_id: JobId,
        reason: String,
    ) -> Result<AdHocOutcome, ToolError> {
        self.stop_job_ad_hoc_with_window(job_id, reason, AD_HOC_PREVIEW_WINDOW)
            .await
    }

    /// Caller-supplied-window variant. Production keeps `window` at
    /// `AD_HOC_PREVIEW_WINDOW`; the e2e suite passes a sub-second
    /// duration so neither test blocks for the full five seconds.
    pub async fn stop_job_ad_hoc_with_window(
        &self,
        job_id: JobId,
        reason: String,
        window: Duration,
    ) -> Result<AdHocOutcome, ToolError> {
        // Subscribe before the preview row so the preview's own
        // `ChatMessageAppended` is not in the stream we are scanning —
        // we are only interested in messages that arrive after the
        // preview, since those represent the user's reaction to it.
        // Filtering by role inside `wait_for_user_wait` handles the
        // belt-and-braces case where the broadcast lag delivers the
        // preview anyway.
        let bus = self.bus_arc();
        let mut stream = bus
            .subscribe_since(SubscribeFilter::Job(job_id), None)
            .await
            .map_err(ToolError::Db)?;

        let preview_body = format!(
            "I'm about to stop this job in {}s: {reason}. Reply 'wait' to cancel.",
            window.as_secs().max(1),
        );
        let preview_metadata = serde_json::json!({
            "preview": {
                "window_ms": window.as_millis() as u64,
                "action": "stop_job",
                "resolves_at": now_ms().0.saturating_add(window.as_millis() as i64),
            }
        })
        .to_string();
        let preview = post_supervisor_row(
            self,
            job_id,
            ChatRole::System,
            preview_body,
            Some(preview_metadata),
            None,
        )
        .await?;

        let aborted = tokio::select! {
            _ = tokio::time::sleep(window) => false,
            outcome = wait_for_user_wait(&mut stream) => outcome,
        };

        if aborted {
            let body = format!(
                "Standing down on the stop — saw a 'wait' inside the {}s window.",
                window.as_secs().max(1),
            );
            let metadata = serde_json::json!({ "resolves": preview.id.to_string() }).to_string();
            post_supervisor_row(
                self,
                job_id,
                ChatRole::Assistant,
                body,
                Some(metadata),
                None,
            )
            .await?;
            return Ok(AdHocOutcome::Aborted);
        }

        self.stop_job_inner(job_id).await?;
        let summary = format!(
            "Stopped the job: {reason} (no 'wait' came in during the {}s preview).",
            window.as_secs().max(1),
        );
        let metadata = serde_json::json!({ "resolves": preview.id.to_string() }).to_string();
        post_supervisor_row(
            self,
            job_id,
            ChatRole::Assistant,
            summary,
            Some(metadata),
            None,
        )
        .await?;
        Ok(AdHocOutcome::Fired)
    }

    /// Append a note row to the chat thread. The row carries
    /// `metadata_json.note = true` so a future notes pane can filter
    /// for these without re-parsing bodies. JOB-WORKFLOW (A) will land
    /// the on-disk `runs/<job_id>/notes/<ts>-supervisor.md` mirror; for
    /// now the chat row is the audit trail and the `ChatMessageAppended`
    /// fan-out is the cross-transport signal.
    pub async fn add_job_note(
        &self,
        job_id: JobId,
        body: String,
    ) -> Result<ChatMessage, ToolError> {
        let metadata = serde_json::json!({ "note": true }).to_string();
        post_supervisor_row(self, job_id, ChatRole::System, body, Some(metadata), None).await
    }

    async fn stop_job_inner(&self, job_id: JobId) -> Result<(), ToolError> {
        // Replicates the state-transition body of `rpc::jobs::stop_job`
        // verbatim. Keeping the action tool inside `codeless-runtime`
        // (same crate as the RPC) means a future change to that
        // transition has to update both sites — the two-call structure
        // forces the reviewer to notice. Routing through the actual
        // `RpcServer::stop_job` would require holding an `Arc<InProcessRpc>`
        // inside `Tools`, which would couple the supervisor's tool
        // surface to the full RPC server type for no observable gain.
        let store = self.store_arc();
        let Some(mut job) = store.get_job(job_id).await.map_err(ToolError::Db)? else {
            return Err(ToolError::NotFound);
        };
        match job.status {
            JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped => {
                // Race against the user clicking the UI button: if the
                // Run is already terminal the action is a no-op success.
                // The follow-up summary still posts so the chat thread
                // records that the supervisor reacted to its trigger,
                // even when the underlying state moved out from under it.
                return Ok(());
            }
            _ => {}
        }
        let now = now_ms();
        job.status = JobStatus::Stopped;
        job.stop_reason = Some(StopReason::User);
        job.ended_at = Some(now);
        store.update_job(&job).await.map_err(ToolError::Db)?;
        self.bus_arc()
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobStopped {
                    job_id: job.id,
                    reason: StopReason::User,
                },
                now,
            )
            .await
            .map_err(ToolError::Db)?;
        Ok(())
    }
}

/// Scan the live event stream for the first `ChatMessageAppended` from
/// a `User`-role row whose body matches `/^wait\b/i` (after trimming
/// leading whitespace). Returns `true` on a match; returns `false` only
/// if the stream ends — the caller wraps this in a `tokio::select!`
/// against the preview-window timer, so the normal exit path for "no
/// wait came in" is the timer firing, not this function returning.
async fn wait_for_user_wait(stream: &mut EventStream) -> bool {
    while let Some(item) = stream.next().await {
        let Ok(env) = item else { continue };
        let Event::ChatMessageAppended { message, .. } = env.event else {
            continue;
        };
        if !matches!(message.role, ChatRole::User) {
            continue;
        }
        if is_wait_prefix(&message.body) {
            return true;
        }
    }
    false
}

/// The `/^wait\b/i` matcher. Implemented by hand rather than via a
/// regex dependency: the supervisor surface is in the hot path of
/// every chat append, and pulling `regex` in for a single fixed
/// pattern would be the wrong trade.
fn is_wait_prefix(body: &str) -> bool {
    let trimmed = body.trim_start().to_ascii_lowercase();
    let Some(rest) = trimmed.strip_prefix("wait") else {
        return false;
    };
    match rest.chars().next() {
        // Word-boundary at end-of-string: bare `wait` matches.
        None => true,
        // Word-boundary against the next char: anything that is not a
        // word character (alnum or `_`) terminates the prefix.
        Some(c) => !c.is_ascii_alphanumeric() && c != '_',
    }
}

/// Shared insert + publish helper. Mirrors `Tools::post_chat_message`
/// but accepts a `role` and `metadata_json` so the action tools can
/// emit `System`-role preview rows and pair follow-ups via
/// `metadata.resolves`.
async fn post_supervisor_row(
    tools: &Tools,
    job_id: JobId,
    role: ChatRole,
    body: String,
    metadata_json: Option<String>,
    external_id: Option<String>,
) -> Result<ChatMessage, ToolError> {
    let store = tools.store_arc();
    if store
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
        external_id,
        thread_key: None,
        author: "supervisor".to_string(),
        role,
        body,
        metadata_json,
        created_at: now,
    };
    match store
        .insert_chat_message(&msg)
        .await
        .map_err(ToolError::Db)?
    {
        InsertChatMessage::Inserted => {}
        InsertChatMessage::DuplicateExternalId => return Err(ToolError::DuplicateInsert),
    }
    let event = Event::ChatMessageAppended {
        job_id,
        message: msg.clone(),
    };
    tools
        .bus_arc()
        .publish(Some(job_id), None, None, event, now)
        .await
        .map_err(ToolError::Db)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_prefix_matches_bare_wait() {
        assert!(is_wait_prefix("wait"));
        assert!(is_wait_prefix("WAIT"));
        assert!(is_wait_prefix("  wait"));
    }

    #[test]
    fn wait_prefix_matches_with_punctuation_or_whitespace() {
        assert!(is_wait_prefix("wait!"));
        assert!(is_wait_prefix("wait a sec"));
        assert!(is_wait_prefix("Wait, please don't"));
    }

    #[test]
    fn wait_prefix_rejects_when_no_word_boundary() {
        assert!(!is_wait_prefix("waiter"));
        assert!(!is_wait_prefix("waiting"));
        assert!(!is_wait_prefix("wait_for_it"));
    }

    #[test]
    fn wait_prefix_rejects_when_not_at_start() {
        assert!(!is_wait_prefix("please wait"));
        assert!(!is_wait_prefix("hold on, wait"));
    }
}
