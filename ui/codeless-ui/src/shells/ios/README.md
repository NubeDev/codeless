# shells/ios

Tauri mobile iOS entry — Phase 6. Rust-side scaffolding lives in
the future `crates/codeless-tauri-mobile/` (reserved per SCOPE.md).

Responsibilities:

1. Construct an `HttpSseClient` pointed at the user's chosen
   codeless-server host (their home box, VPS, or Mac mini).
2. Inject iOS-native capability adapters via Tauri plugins
   (clipboard, share sheet, biometric unlock for the bearer token).
3. Mount `<App rpc={...} caps={...} />`.

The UI tree is byte-identical to browser and android shells — only
this entry file and the injected capability adapters are
iOS-specific.
