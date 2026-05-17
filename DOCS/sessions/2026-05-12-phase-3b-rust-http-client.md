# Build status — Phase 3b Rust HTTP RPC client (`codeless-client`)

> Agent context: `codeless-client` is the iOS-safe / Android-safe
> sibling of the browser's `HttpSseClient`. It implements `RpcServer`
> over the same `/rpc/<method>` REST + `/events` SSE wire that
> `codeless-server` exposes (locked by
> `ui/codeless-ui/src/lib/rpc/http-sse-client.ts`). The mobile
> shells in Phase 6 will reach this crate transitively; today, the
> CLI's `--core` mode is the first consumer.

File: DOCS/sessions/2026-05-12-phase-3b-rust-http-client.md
Goal: A Rust `HttpRpcClient` that implements the full `RpcServer`
      trait against a running `codeless-server`, plus a first CLI
      consumer (`codeless repos list --core <url> --token <t>`)
      proving the round-trip end-to-end against a real server.

Repo:     codeless
Branch:   feat/phase-2a-persistence (stacked; combined PR cuts later)

Status: DONE — all 3 stages [x]. 14 client crate tests + 3
hosted-CLI integration tests green; combined workspace at 135
tests pass / 0 fail, clippy `-D warnings` clean, fmt clean.

## Stages

- [x] 1. [M] `codeless-client` REST surface. Add deps (`async-trait`,
         `reqwest` rustls, `serde`, `serde_json`, `futures-core`,
         `thiserror`). Implement `HttpRpcClient` with the 10
         non-subscribe `RpcServer` methods over `POST /rpc/<m>` +
         bearer header. Subscribe returns an error stream stub
         marked TODO for stage 2. Map HTTP status → `RpcError`
         variants matching `routes::map_err`. Tests: in-process axum
         router via `tokio::net::TcpListener` on ephemeral port +
         the new client end-to-end.

- [x] 2. [M] SSE subscribe. Parse `text/event-stream` from a
         streaming reqwest body into `EventEnvelope`s. Honour
         `Last-Event-ID` semantics (the cursor in the SSE `id:`
         line drives a `?since=` query on reconnect). Live + replay
         tests against the same axum harness.

- [x] 3. [S] CLI `codeless repos list --core URL --token T` verb.
         When `--core` is set, dispatch picks `HttpRpcClient`;
         otherwise the existing in-process path. Other verbs stay
         in-process-only; trying them with `--core` is a clean error.
         Integration test: spawn `codeless serve`, then exec
         `codeless --core ... --token ... repos list` and assert
         the seeded repo is in stdout.

## Notes
- R1: `reqwest` with `rustls-tls` is mobile-safe; `tokio::process`
  is not — keep the CLI's spawn-y bits out of the client crate.
- The wire shape `routes::map_err` defines is the spec; the client
  must decode the same way or RpcError variants drift between the
  in-process and HTTP paths.
- This phase is a stacked extension of the Phase 3a branch; the PR
  draft picks it up by reference.

## Outcomes
- `HttpRpcClient` implements every `RpcServer` method including
  `subscribe`. REST goes through reqwest with bearer; SSE parses
  `text/event-stream` via a hand-rolled `SseParser` (handles chunk
  boundaries, CRLF, comment lines, `event: error` frames).
- `status_to_rpc` mirrors `codeless-server::routes::map_err`
  exactly — the round-trip test catches drift between the two
  variants if either side renames or remaps.
- New `codeless repos list` verb is dual-mode: `--core URL --token T`
  → `HttpRpcClient`; otherwise the in-process path. `--token`
  without `--core` is a clean error.
- `--core` / `--token` are global flags backed by `CODELESS_CORE`
  / `CODELESS_TOKEN` env vars so tokens never need to land in
  shell history.

## Blockers
(none)
