use thiserror::Error;

/// Failure modes a tool call may surface.
///
/// `Cancelled` is structurally distinct from `Failed` so the MCP
/// dispatch layer can map it to the runner's cancellation protocol
/// without inspecting the message — runners need to distinguish "I
/// asked it to stop" from "it crashed."
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool input invalid: {0}")]
    InvalidArgs(String),

    #[error("tool was cancelled before completion")]
    Cancelled,

    #[error("tool denied by policy: {0}")]
    Denied(String),

    #[error("tool execution failed: {0}")]
    Failed(String),
}

impl ToolError {
    pub fn invalid_args(msg: impl Into<String>) -> Self {
        Self::InvalidArgs(msg.into())
    }

    pub fn denied(msg: impl Into<String>) -> Self {
        Self::Denied(msg.into())
    }

    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }
}
