// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/browser/protocol.rs
//
// Line-delimited JSON-RPC over stdio. The Rust side speaks this to
// the Node sidecar (`sidecars/playwright/sidecar.mjs`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcResponse {
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}
