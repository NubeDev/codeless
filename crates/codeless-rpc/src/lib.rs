// The RPC trait and transport-agnostic client glue — see the
// `codeless-rpc` row of the crate table in DOCS/SCOPE.md. No
// assumptions about how bytes move (SSE, REST, WebSocket, Tauri IPC
// all implement the same trait). iOS- and Android-safe.
