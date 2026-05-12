# shells/

Per-shell entry points. Each shell builds an `RpcClient`
implementation and mounts the **same** `<App>` from
[`../app/`](../app). No shell contains components; no component
contains shell-specific code. The split is the seam SCOPE.md "One UI,
four shells" calls out.

| Shell | Entry | RpcClient implementation | Phase |
|---|---|---|---|
| [`browser/`](./browser) | `main.tsx` mounted by `index.html`, served by `codeless-server` | `HttpSseClient` (REST + SSE) | 1 (MVP) |
| [`desktop/`](./desktop) | Tauri 2 webview entry | `TauriIpcClient` (`invoke()` + event listener) | 5 |
| [`ios/`](./ios) | Tauri mobile webview entry | `HttpSseClient` over the user's chosen host | 6 |
| [`android/`](./android) | Tauri mobile webview entry | `HttpSseClient` over the user's chosen host | 6 |

iOS and Android share the same `RpcClient` and run identical UI code;
they are split into sibling dirs because each Tauri-mobile target has
its own platform-specific build config (signing, icons, native
plugins). When a difference is genuinely platform-only it lives in a
**Tauri plugin** in Rust, never in a `Foo.ios.tsx` file.

See workspace [`CLAUDE.md`](../../../../CLAUDE.md) R2/R3 and
[`DOCS/SCOPE.md`](../../../../DOCS/SCOPE.md) "One UI, four shells".
