use serde::{Deserialize, Serialize};

use crate::id::{JobId, MessageId};
use crate::time::UnixMillis;

/// Origin surface of a chat row. The wire form is lowercase ASCII on
/// every transport (JSON, SQLite `transport` column, Telegram/Slack
/// metadata, log fields) — see `DOCS/JOB-CHAT.md` "Wire-name
/// convention for `ChatTransport`". Adapters compare values only as
/// these lowercase strings; never as Rust identifiers, display names,
/// or human-language synonyms.
///
/// The v0.1 surface is closed at five variants. `Web`, `Cli`, and
/// `Supervisor` ship with the C1 substrate; `Telegram` and `Slack`
/// arrive with the first transport adapter. The supervisor agent is
/// modelled as a transport rather than a role so a single
/// `(transport, external_id)` self-match could in principle suppress
/// echo back to a participant — though the actual rule in JOB-CHAT.md
/// is asymmetric and lives in the bot-core helper, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ChatTransport {
    Web,
    Cli,
    Telegram,
    Slack,
    Supervisor,
}

/// Speaker class of a chat row. Mirrors the role column on
/// `chat_messages` and overlaps deliberately with
/// `AssistantMessageRole` so the same CommonChat renderer can paint
/// both surfaces. Kept as its own enum (rather than reusing the
/// assistant one) because the two surfaces evolve independently —
/// JOB-CHAT.md is explicit that the per-Job chat is not the
/// `/assistant` surface, even though they look similar in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    Tool,
    System,
}

/// One persisted row in `chat_messages`. The single conversation
/// every transport reads and writes — see `DOCS/JOB-CHAT.md` "Data
/// model".
///
/// `run_id` is currently typed as `Option<String>` because the
/// JOB-WORKFLOW (B) Job/Run split (and the `RunId` newtype it
/// introduces) has not landed yet; the SQL column is already nullable
/// and rows minted before (B) leave it NULL. Once (B) lands, this
/// field swaps to `Option<RunId>` without a wire-format change — the
/// transparent newtype serialises as the same JSON string.
///
/// `external_id` is NOT NULL on the SQL side for Telegram and Slack
/// rows (the partial UNIQUE index enforces ingest idempotency for
/// those transports); the wire type carries `Option<String>` because
/// web, CLI, and supervisor rows leave it NULL and the column is
/// generic. The invariant is owned by the runtime insert path, not
/// the wire type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChatMessage {
    pub id: MessageId,
    pub job_id: JobId,
    pub run_id: Option<String>,
    pub transport: ChatTransport,
    pub external_id: Option<String>,
    pub thread_key: Option<String>,
    pub author: String,
    pub role: ChatRole,
    pub body: String,
    /// Transport-specific extras (attachments, formatting, delivery
    /// receipts written by outbound forwarders). Carried as the raw
    /// JSON text exactly as stored in the SQL `metadata_json` column
    /// — keeping the field a `String` avoids pulling `serde_json`
    /// into `codeless-types`, which has to compile for mobile (R1).
    /// Callers that need a structured view parse it themselves on
    /// arrival; the substrate stays opaque.
    pub metadata_json: Option<String>,
    pub created_at: UnixMillis,
}

/// One row in `chat_bindings`: the lookup that turns an inbound
/// message on a transport's `(channel, thread)` into the Job that
/// owns the conversation. Written by `/codeless bind` on the
/// transport surface; read by the inbound adapter before it calls
/// `post_job_message`. The web UI never needs a binding — it already
/// knows the Job id from its URL.
///
/// `thread_id` uses the empty string `""` as the sentinel for "no
/// thread on this transport" rather than `Option<String>`: the
/// primary key on the SQL side is `(transport, channel_id,
/// thread_id)`, and SQLite treats every NULL as distinct in a UNIQUE
/// constraint, which would silently allow two different jobs to bind
/// to the same `(transport, channel)` if `thread_id` were nullable.
/// The empty string is unambiguous on both Telegram and Slack — no
/// real thread id is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChatBinding {
    pub transport: ChatTransport,
    pub channel_id: String,
    pub thread_id: String,
    pub job_id: JobId,
    pub bound_at: UnixMillis,
    pub bound_by: String,
}
