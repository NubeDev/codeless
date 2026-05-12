# shells/browser

Browser entry point — the MVP shell. Loaded by the root
`index.html`, served by [`codeless-server`](../../../../crates/codeless-server)
as static assets.

Responsibilities:

1. Construct an `HttpSseClient` (REST + Server-Sent Events) pointed
   at the codeless-server origin, with the bearer token from
   `localStorage` (or an initial setup flow).
2. Inject shell-side capability adapters (clipboard via the Web
   Clipboard API, file picker via `<input type="file">`, etc).
3. Mount `<App rpc={...} caps={...} />`.

Does **not** import `@tauri-apps/api/*` or contain UI components.
