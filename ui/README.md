# ui/

Frontend packages for Codeless. Each subdirectory is its own
package; the canonical one is [`codeless-ui/`](./codeless-ui).

## Packages

- [`codeless-ui/`](./codeless-ui) — the single React + TypeScript UI
  that ships to all four shells (browser, Tauri desktop, iOS,
  Android). Imports `RpcClient` only — never `@tauri-apps/api/core`,
  never `fetch()` to the codeless server directly. Shell-specific
  behaviour is injected via interfaces, not per-shell files.

See [`../CLAUDE.md`](../CLAUDE.md) R2/R3 for the cross-platform
boundary rules these packages must respect.
