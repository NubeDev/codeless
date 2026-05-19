use codeless_types::{AdapterError, WorkspaceError};
use thiserror::Error;

/// Errors returned by every `RpcServer` method. Variants are wire-stable:
/// transports map them to HTTP status codes (REST), close codes (WS), or
/// `Result::Err` (in-process / Tauri IPC). Renaming a variant is a
/// breaking change.
#[derive(Debug, Error)]
pub enum RpcError {
    /// The referenced row does not exist (or is no longer reachable).
    #[error("not found: {0}")]
    NotFound(String),

    /// Caller supplied invalid arguments — wrong shape, out-of-range, etc.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The runtime refused the request because of its current state
    /// (e.g. stopping a job that is already completed).
    #[error("conflict: {0}")]
    Conflict(String),

    /// Structured workspace failure. Carries a typed `WorkspaceError`
    /// payload so the UI can branch on the variant (`AlreadyAttached`,
    /// `RunningJobs`, `PathRejected`, `NotAttached`) without string
    /// matching on a generic `Conflict` message. Transports serialise
    /// the payload as JSON in the response body — see
    /// `WORKSPACE-ATTACH.md` §"Error model".
    #[error("workspace: {0:?}")]
    Workspace(WorkspaceError),

    /// Structured adapter-registry failure (`MissingSecrets`,
    /// `ValidationFailed`, `RestartUnsupervised`,
    /// `RestartHasRunningJobs`, `Conflict`, `NotConfigured`). The UI
    /// branches on the variant to drive the Settings → Adapters page
    /// and the restart confirm modal. See `DOCS/WORKSPACE-ATTACH.md`
    /// §"TODO — adapter registry".
    #[error("adapter: {0:?}")]
    Adapter(AdapterError),

    /// Anything the runtime can't express more specifically. Transports
    /// surface this as a generic 500. Avoid in new code — add a variant.
    #[error("internal: {0}")]
    Internal(String),
}

pub type RpcResult<T> = Result<T, RpcError>;
