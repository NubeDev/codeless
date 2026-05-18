use thiserror::Error;

/// Errors a [`crate::ToolBehavior::call`] may return.
///
/// The variants line up with `tool-error.code` in `tool.wit`
/// (PLUGIN-WASM.md) so the host-side translator at the WASM
/// boundary is a flat match, not a string-parse. Adding a variant
/// here is an ABI change for the wasm flavour -- it requires the
/// matching WIT update and a re-generation of the committed
/// bindings under `crates/codeless-tool-wit/` (OQ-WASM-2).
#[derive(Debug, Error)]
pub enum ToolError {
    /// The args object failed the tool's own precondition checks.
    /// Distinct from a JSON-schema validation failure (which the
    /// runner catches before `call`); use this for semantic
    /// violations the schema cannot encode.
    #[error("invalid args: {0}")]
    InvalidArgs(String),

    /// The call could not complete. Maps to `tool-error.retryable =
    /// false` -- the agent loop should report the failure to the
    /// user rather than silently retry.
    #[error("failed: {0}")]
    Failed(String),

    /// Transient failure the runner may retry once. Maps to
    /// `tool-error.retryable = true`.
    #[error("retryable: {0}")]
    Retryable(String),

    /// The host cancelled the call through `ToolCtx`. Surfaced as a
    /// distinct variant so the dispatcher can distinguish a
    /// cooperatively-cancelled tool from a genuinely-failed one in
    /// telemetry.
    #[error("cancelled")]
    Cancelled,
}

impl ToolError {
    pub fn invalid_args(msg: impl Into<String>) -> Self {
        Self::InvalidArgs(msg.into())
    }

    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }

    pub fn retryable(msg: impl Into<String>) -> Self {
        Self::Retryable(msg.into())
    }

    /// Stable string code matching `tool-error.code` in `tool.wit`.
    /// The wasm-flavour adapter reads this to fill the WIT field;
    /// the builtin-flavour adapter ignores it.
    pub const fn code(&self) -> &'static str {
        match self {
            ToolError::InvalidArgs(_) => "invalid-args",
            ToolError::Failed(_) => "failed",
            ToolError::Retryable(_) => "retryable",
            ToolError::Cancelled => "cancelled",
        }
    }

    pub const fn retryable_flag(&self) -> bool {
        matches!(self, ToolError::Retryable(_))
    }
}
