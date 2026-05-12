# codeless-ui

The single React + TypeScript UI for Codeless. Same component tree
ships to browser (MVP), Tauri desktop, iOS, and Android.

## Develop

```sh
cd codeless/ui/codeless-ui
pnpm install
pnpm dev
```

`pnpm build` runs `tsc && vite build`.

## Boundary rules

- Import `RpcClient` only. **Never** import `@tauri-apps/api/core`,
  `@tauri-apps/api/event`, or `fetch()` to the codeless server.
- The shell decides which `RpcClient` implementation to inject —
  `HttpSseClient` (browser/mobile) or `TauriIpcClient` (desktop).
- No per-shell files. No `Foo.web.tsx`, no `Foo.mobile.tsx`.
  Responsive design + shell-injected interfaces (clipboard, file
  picker, biometric) cover every per-platform difference.

Full rationale: workspace [`CLAUDE.md`](../../../CLAUDE.md) R2/R3 and
[`DOCS/SCOPE.md`](../../../DOCS/SCOPE.md) "One UI, four shells".

## Origin

Initial source ported from [`crynta/terax-ai`](https://github.com/crynta/terax-ai)
(Apache-2.0). Attribution and the pinned upstream SHA are in
[`NOTICE.md`](./NOTICE.md). Codeless does not maintain upstream
compatibility — the port is a one-way absorb.
