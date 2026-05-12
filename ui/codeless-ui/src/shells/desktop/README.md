# shells/desktop

Tauri 2 desktop entry — Phase 5. The Rust side lives in
[`../../../../crates/codeless-tauri-desktop/`](../../../../crates/codeless-tauri-desktop).

Responsibilities:

1. Construct a `TauriIpcClient` that wraps `invoke()` for
   request/reply and the Tauri event API for the subscription
   stream.
2. Inject desktop-native capability adapters (clipboard plugin,
   file dialog plugin, OS keychain via `RpcSecrets`).
3. Mount `<App rpc={...} caps={...} />`.

Same `<App>` as browser/mobile — only the injected `RpcClient` and
capability adapters differ. No component imports
`@tauri-apps/api/core` directly; everything goes through the
injected interfaces.
