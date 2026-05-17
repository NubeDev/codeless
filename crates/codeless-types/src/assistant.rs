use serde::{Deserialize, Serialize};

use crate::id::{AssistantAttachmentId, AssistantMessageId, AssistantThreadId, JobId, RepoId};
use crate::job::WorkspaceMode;
use crate::time::UnixMillis;

/// One conversational thread on the `/assistant` surface — see
/// `DOCS/ASSISTANT-SCOPE.md`. Threads outlive any single job/worktree
/// and therefore have no foreign key onto `repos` or `jobs`; the
/// assistant is allowed to span jobs by design (Decisions §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantThread {
    pub id: AssistantThreadId,
    pub title: String,
    /// Persona this thread runs under, declared at creation and
    /// immutable for the thread lifetime (PS5, see
    /// `DOCS/PLUGIN-SUBSTRATE.md` item 5). The runner reads
    /// `system_prompt`, `allowed_tools`, `default_model_family`, and
    /// `default_attachments_policy` off the persona row keyed by this
    /// id at agent-call time. The column is NOT NULL on the SQLite
    /// side and `create_assistant_thread` returns
    /// `InvalidArgument` when the caller omits it -- there is no
    /// silent fallback.
    pub persona_id: String,
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

/// Proposed view/manage tool call, stored inside an assistant turn's
/// `meta_json` as a JSON document (see `AssistantActionCard::META_KIND`).
/// Each variant corresponds 1:1 to the `RpcServer` method the runtime
/// will dispatch on confirmation; only the args the tool actually
/// needs are captured here so the wire form stays narrow enough for a
/// human to read in a diff.
///
/// The `restart` alias maps to `rerun_job` because there is no separate
/// `restart_job` RPC — the project uses "rerun" for the clean-attempt
/// flow, but the assistant scope (stage 7) and chat UX speak in terms
/// of "restart". The alias lives here, not in the RPC layer, so the
/// server surface stays unsurprising for non-assistant callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "tool")]
pub enum AssistantAction {
    ListJobs {
        #[serde(default)]
        repo_id: Option<RepoId>,
    },
    GetJob {
        job_id: JobId,
    },
    StartJob {
        job_id: JobId,
    },
    StopJob {
        job_id: JobId,
    },
    PauseJob {
        job_id: JobId,
    },
    ResumeJob {
        job_id: JobId,
    },
    /// "Restart" in chat is "rerun" in the RPC surface — same caps and
    /// prompt, fresh `JobId` and branch. Naming preserved so the UI
    /// can keep the user-facing verb without the back-end gaining a
    /// duplicate method.
    RestartJob {
        job_id: JobId,
    },
    /// Partial-update of a non-running job. Every field is optional;
    /// `None` means "leave unchanged". The chat parser only fills the
    /// fields the user mentioned, so a confirmation card stays a thin
    /// patch instead of a full echo of the job row.
    UpdateJob {
        job_id: JobId,
        #[serde(default)]
        runner: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        permission_mode: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        cost_cap_cents: Option<i64>,
        #[serde(default)]
        wall_clock_cap_ms: Option<i64>,
        #[serde(default)]
        branch: Option<String>,
    },
    /// Propose a new job. Stage-8 "draft from conversation": the
    /// planner (or its slash-command stand-in) folds the user's
    /// request into a fully-specified `submit_job` payload that the
    /// user reviews and confirms. Confirmation dispatches `submit_job`
    /// with `start_immediately = false` so the row lands in `Draft`
    /// (SCOPE.md Decisions §3 — no "just do it" path).
    ///
    /// Every field that drives `SubmitJobArgs` is captured explicitly
    /// here so the confirmation card is a complete review of what
    /// will be created. Defaults applied by the parser are still
    /// surfaced on the card — the user sees exactly what they are
    /// approving instead of guessing what is implicit.
    DraftJob {
        repo_id: RepoId,
        prompt: String,
        runner: String,
        branch: String,
        cost_cap_cents: i64,
        wall_clock_cap_ms: i64,
        #[serde(default)]
        workspace_mode: Option<WorkspaceMode>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        permission_mode: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        /// Hands-off advancement policy applied if a stage fails.
        /// Mirrors the picker in `SubmitJobDialog` (Surface F) so a
        /// job drafted from chat carries the same opt-in the form
        /// surface offers. `None` keeps the default — a stage failure
        /// halts the job and waits for the operator.
        #[serde(default)]
        auto_bypass_policy: Option<crate::AutoBypassPolicy>,
    },
    /// Rewrite one of the job's spec files (default `SCOPE.md`) with
    /// new content. Stage-9 "edit-scope": the chat surface proposes
    /// the full new file body; confirmation dispatches `write_job_file`
    /// after the paused-job guard runs (Running / Queued /
    /// AwaitingReview rows refuse the edit — the user pauses the job
    /// first). The card stores the full proposed body so the
    /// renderer can compute a unified diff against the current file
    /// on the fly; only the filename and body cross the wire, so
    /// `meta_json` stays a flat document a human can read in `git log`.
    EditScope {
        job_id: JobId,
        /// Target file under `<repo>/.codeless/jobs/<name>/`. Defaults
        /// to `SCOPE.md` at the parser layer; carried explicitly here
        /// so a future planner can target `WORKFLOW.md` (or another
        /// non-template file) without a second action variant. The
        /// server rejects `template.yaml` — `update_job_template` is
        /// the correct path for the spec (renames refused there too).
        filename: String,
        new_content: String,
    },
    /// Change the per-job auto-bypass policy. Mirrors the picker the
    /// `SubmitJobDialog` exposes at submit time (`AutoBypassPolicy`),
    /// so the assistant can propose policy changes the same way a
    /// human would set them in the form: "if this stage fails again,
    /// switch the job to long-term so it auto-recovers."
    ///
    /// `policy: None` clears the policy entirely (the job reverts to
    /// the default halt-on-failure behaviour). The underlying
    /// `set_job_policy` RPC refuses the change while the job is
    /// `Running` or `Queued` (`AUTO-BYPASS-DECISIONS.md` Q5); the
    /// chat surface inherits that guard — a confirmation against a
    /// running job surfaces as a `Conflict` on the resolved card.
    SetPolicy {
        job_id: JobId,
        #[serde(default)]
        policy: Option<crate::AutoBypassPolicy>,
    },
}

impl AssistantAction {
    /// Whether confirming this card mutates server state. The UI uses
    /// this to label the confirm button ("Run" for read-only,
    /// "Confirm & run" for mutating) and to colour the card border.
    /// Centralised here so a new variant cannot forget to declare its
    /// blast radius — adding one without updating the match arm is a
    /// compile error.
    pub fn mutates(&self) -> bool {
        match self {
            AssistantAction::ListJobs { .. } | AssistantAction::GetJob { .. } => false,
            AssistantAction::StartJob { .. }
            | AssistantAction::StopJob { .. }
            | AssistantAction::PauseJob { .. }
            | AssistantAction::ResumeJob { .. }
            | AssistantAction::RestartJob { .. }
            | AssistantAction::UpdateJob { .. }
            | AssistantAction::DraftJob { .. }
            | AssistantAction::EditScope { .. }
            | AssistantAction::SetPolicy { .. } => true,
        }
    }
}

/// Lifecycle of an action card. Stored as the `status` field inside
/// the card's `meta_json` document so the chat history captures both
/// the original proposal and what the user did with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantActionStatus {
    /// Awaiting the user's confirm/cancel click.
    Pending,
    /// Confirmed and dispatched; a subsequent `Tool` message carries
    /// the structured result.
    Confirmed,
    /// User cancelled — no RPC fired, no state changed.
    Cancelled,
    /// Confirmed but the dispatched RPC returned an error. The error
    /// summary lives on the trailing `Tool` message.
    Failed,
}

/// The structured payload of an `Assistant`-role message that
/// proposes a tool call. Serialised into `AssistantMessage.meta_json`
/// with `kind == META_KIND` so the renderer can discriminate between
/// a plain markdown reply (NULL meta or unknown kind) and an action
/// card without a separate column on the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantActionCard {
    /// Discriminator. Always [`AssistantActionCard::META_KIND`]; the
    /// field exists so a future `meta_json` variant (e.g. attachment
    /// preview, draft-job card) can coexist on the same column.
    pub kind: String,
    pub status: AssistantActionStatus,
    pub action: AssistantAction,
}

impl AssistantActionCard {
    pub const META_KIND: &'static str = "action_card";

    pub fn new(action: AssistantAction) -> Self {
        Self {
            kind: Self::META_KIND.to_owned(),
            status: AssistantActionStatus::Pending,
            action,
        }
    }
}

/// One attachment reference as returned by a plugin tool (PS7,
/// `DOCS/PLUGIN-SUBSTRATE.md` item 7). `attachment_id` is the
/// authoritative server-minted id of an existing row in
/// `assistant_attachments`; `mime` and `filename` are *advisory* hints
/// the renderer may use as a fast path. The substrate-doc rule is that
/// the server reconciles every advisory hint against the stored row
/// (`reconcile_attachment_refs`) and the stored value wins -- a tool
/// that mistypes the mime or renames the file in flight cannot trick
/// the renderer into a bogus content-type or path.
///
/// Tools opt into this shape by declaring their output schema with the
/// `{"$ref": "codeless://attachment"}` marker (see
/// `codeless_tools::attachment::ATTACHMENT_SCHEMA_REF`). A tool that
/// returns multiple attachments returns an array of these objects or
/// wraps them in a field whose schema is the array form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AttachmentRef {
    pub attachment_id: AssistantAttachmentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// One reconciled attachment item the UI renders inside an
/// [`AssistantAttachmentCard`]. Fields are populated from the stored
/// `assistant_attachments` row (authoritative); any tool-supplied
/// hint that disagrees was dropped during reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantAttachmentCardItem {
    pub attachment_id: AssistantAttachmentId,
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: i64,
}

/// Structured payload of an `Assistant`- or `Tool`-role message whose
/// `meta_json` declares one or more reconciled attachments produced by
/// a tool call (PS7). The renderer dispatches on `kind ==
/// META_KIND` and shows a download card (plus inline preview for
/// recognised mime types) without any per-plugin UI code.
///
/// Lives next to [`AssistantActionCard`] so the two meta-kind variants
/// the substrate carries on `assistant_messages.meta_json` share a
/// header and a single `kind` discriminator namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AssistantAttachmentCard {
    /// Discriminator. Always [`AssistantAttachmentCard::META_KIND`];
    /// the UI parser checks this before deserialising the rest.
    pub kind: String,
    pub items: Vec<AssistantAttachmentCardItem>,
}

impl AssistantAttachmentCard {
    pub const META_KIND: &'static str = "attachment_card";

    pub fn new(items: Vec<AssistantAttachmentCardItem>) -> Self {
        Self {
            kind: Self::META_KIND.to_owned(),
            items,
        }
    }
}
