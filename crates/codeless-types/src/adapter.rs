//! Wire types for the adapter registry — the SQLite-backed set of
//! enabled chat adapters (Slack, Telegram) and AI runners (`claude`,
//! `anthropic`, `codex`, `copilot`) that replaces the boot-time
//! `--enable-*` CLI flags. Authoritative design lives in
//! `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry"; the
//! per-job brief in `.codeless/jobs/adapter-registry/SCOPE.md`
//! names every type below.
//!
//! These types are pure data and ride the standard RPC channel
//! behind the bearer gate. Kept in `codeless-types` so the mobile
//! shells pick them up via `codeless-client` without dragging in any
//! host-only deps (see SCOPE.md crate layout).
//!
//! `Gmail` is deliberately absent from `ChatAdapterKind`; that
//! adapter ships as the follow-up `codeless-gmail` crate and the
//! variant lands then, paired with its OAuth host wiring.

use serde::{Deserialize, Serialize};

use crate::id::JobId;
use crate::time::UnixMillis;

/// Kinds of chat adapter the registry knows about today. New variants
/// land paired with their crate (`codeless-gmail`, `codeless-discord`,
/// …); the table's `(kind, instance_id)` PK absorbs the addition
/// without a schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ChatAdapterKind {
    Slack,
    Telegram,
}

/// One row in the `chat_adapters` table as exposed on the wire. The
/// composite `(kind, instance_id)` identity lets the user run e.g.
/// Slack-personal + Slack-work side by side; the default
/// `instance_id = "default"` covers the today-case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChatAdapterRow {
    pub kind: ChatAdapterKind,
    pub instance_id: String,
    pub enabled: bool,
    /// When this row last changed enabled/secret state. Lets the UI
    /// render "enabled 3 minutes ago" without a second query.
    pub configured_at: UnixMillis,
}

/// One row in the `runner_config` table. `runner_id` is the same
/// free-form string used elsewhere on the wire (`claude`, `anthropic`,
/// `codex`, `copilot`); kept open so a new runner crate registers a
/// row at boot without a schema change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RunnerRow {
    pub runner_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListChatAdaptersResult {
    pub adapters: Vec<ChatAdapterRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListRunnersResult {
    pub runners: Vec<RunnerRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SetChatAdapterEnabledArgs {
    pub kind: ChatAdapterKind,
    pub instance_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SetRunnerEnabledArgs {
    pub runner_id: String,
    pub enabled: bool,
}

/// Dry-run secret check for one chat-adapter instance. The server
/// hits the upstream's identity endpoint (Slack `auth.test`, Telegram
/// `getMe`) under a 5s hard timeout; the result is cached in-process
/// for the lifetime of the server so a subsequent
/// `set_chat_adapter_enabled(true)` is allowed to proceed. A restart
/// clears the cache and forces re-validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ValidateChatAdapterSecretsArgs {
    pub kind: ChatAdapterKind,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ValidateChatAdapterSecretsResult {
    pub ok: bool,
    /// Populated when `ok = false`. The UI renders each variant inline
    /// without string-matching on a generic message.
    pub problems: Vec<ChatAdapterSecretProblem>,
}

/// Structured failure modes returned by
/// `validate_chat_adapter_secrets`. Distinct from `AdapterError` —
/// these are *observations* the UI renders, not RPC-level failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ChatAdapterSecretProblem {
    /// One or more secret keys are absent from the `SecretStore`.
    /// `keys` carries the canonical `secrets.toml` key names so the
    /// UI can prompt for exactly the missing values.
    MissingSecrets { keys: Vec<String> },
    /// The upstream accepted the request but rejected the credentials.
    Unauthorized { reason: String },
    /// The upstream did not respond inside the 5s timeout.
    Timeout,
    /// Anything else the validator could not classify — network error,
    /// unexpected upstream response shape, etc.
    Other { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RestartServerArgs {
    /// When `false` (default), the call refuses with
    /// `RestartHasRunningJobs` if any job is `Running`, returning the
    /// resumable/killed partition so the UI can render its confirm
    /// modal truthfully. When `true`, the server proceeds and the
    /// `killed` jobs lose their PTY mid-stream.
    #[serde(default)]
    pub force: bool,
}

/// Success here is "you will not see this response" — the connection
/// drops as the process exits. The struct exists so the RPC has a
/// shape and so a future synchronous success path (Tauri desktop
/// returning *before* sidecar respawn) has somewhere to land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RestartServerResult {}

/// Structured failure modes for the six adapter-registry RPCs. Wire-
/// distinct from a generic `Conflict` so the UI branches on the
/// variant — e.g. `MissingSecrets` reopens the secrets fields,
/// `RestartHasRunningJobs` drives the confirm modal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterError {
    /// `set_chat_adapter_enabled(true)` called without a prior
    /// successful `validate_chat_adapter_secrets` for the same
    /// `(kind, instance_id)`, *or* one of the named keys is missing
    /// from the `SecretStore`. Carries the canonical secret-key names
    /// so the UI can prompt for exactly the missing values.
    MissingSecrets { keys: Vec<String> },
    /// `validate_chat_adapter_secrets` ran but the upstream rejected
    /// the credentials (Slack `invalid_auth`, Telegram `401`, etc.).
    /// Cleared on a subsequent successful validation.
    ValidationFailed { reason: String },
    /// `restart_server` called against a `codeless serve` that is not
    /// running under a supervisor (no systemd, no `init-session.sh`,
    /// no `--respawn-on-exit`). The `hint` is a copy-pasteable command
    /// the user can run to restart manually.
    RestartUnsupervised { hint: String },
    /// `restart_server` called with `force = false` while at least one
    /// job is `Running`. `resumable` jobs hold a recent checkpoint and
    /// will continue from it on the next boot; `killed` jobs lose
    /// in-flight PTY output and need an explicit operator decision.
    RestartHasRunningJobs {
        resumable: Vec<JobId>,
        killed: Vec<JobId>,
    },
    /// Generic structural conflict — the requested transition does
    /// not apply to the row's current state.
    Conflict,
    /// The referenced `(kind, instance_id)` or `runner_id` has no row
    /// in the registry yet.
    NotConfigured,
}
