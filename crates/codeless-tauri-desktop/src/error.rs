use codeless_rpc::RpcError;

/// Wrapper around `RpcError` that implements `Serialize` so Tauri can
/// send it over IPC. Tauri 2 commands require `Result<T, E>` where
/// `E: Serialize`; `RpcError` itself derives only `thiserror::Error`.
#[derive(Debug)]
pub struct CommandError(pub RpcError);

impl serde::Serialize for CommandError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl From<RpcError> for CommandError {
    fn from(e: RpcError) -> Self {
        Self(e)
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
