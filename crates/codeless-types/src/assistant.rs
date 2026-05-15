use serde::{Deserialize, Serialize};

use crate::id::{AssistantAttachmentId, AssistantMessageId, AssistantThreadId};
use crate::time::UnixMillis;

/// One conversational thread on the `/assistant` surface — see
/// `DOCS/ASSISTANT-SCOPE.md`. Threads outlive any single job/worktree
/// and therefore have no foreign key onto `repos` or `jobs`; the
/// assistant is allowed to span jobs by design (Decisions §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantThread {
    pub id: AssistantThreadId,
    pub title: String,
    pub created_at: UnixMillis,
    pub updated_at: UnixMillis,
}

/// Who said the message. Kebab-case on the wire to match the rest of
/// the codebase's status enums and the `chat-message` event payload
/// that the CommonChat renderer shares with the live job chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantMessageRole {
    User,
    Assistant,
    /// Runtime-injected context (thread rename, attachment added). The
    /// UI renders these as muted dividers rather than chat bubbles.
    System,
    /// Tool call surface from an action card. The structured payload
    /// lives in `AssistantMessage.meta_json`; `content` is the
    /// human-readable summary the UI falls back to when it cannot
    /// render the card.
    Tool,
}

/// One persisted turn on a thread. `meta_json` mirrors the shape of
/// the `chat-message` event payload so the assistant transcript and
/// the in-job chat can share one renderer (see SCOPE.md Stage 3 —
/// `CommonChat`). NULL meta is the bare-text case the UI renders as
/// plain markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantMessage {
    pub id: AssistantMessageId,
    pub thread_id: AssistantThreadId,
    pub role: AssistantMessageRole,
    pub content: String,
    pub meta_json: Option<String>,
    pub created_at: UnixMillis,
}

/// One file uploaded into a thread. The blob lives under
/// `<codeless-data>/threads/<thread_id>/attachments/<stored_filename>`
/// (SCOPE.md Decisions §1); this row is the durable index the UI
/// renders and the cascade target when `assistant.deleteThread` runs.
/// `stored_filename` is the on-disk basename (id-prefixed for
/// collision-resistance); `original_name` is what the user dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantAttachment {
    pub id: AssistantAttachmentId,
    pub thread_id: AssistantThreadId,
    pub original_name: String,
    pub stored_filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub created_at: UnixMillis,
}
