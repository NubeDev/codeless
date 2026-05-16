# Draft PR — `feat/phase-2a-persistence` (Phases 2a → 3m)

> Not opened automatically; landing the PR is a human action. This
> file is the draft body the human can paste into `gh pr create`.

## Title
`feat: persistence + runtime + reviews + hosted server + dual-mode CLI`

## Summary

Stacked-phases branch bringing Codeless from "Cargo workspace skeleton"
to "the demo works end-to-end against a hosted core." Bundling
because each phase leans on the previous one's wire types and
runtime surface; splitting would either duplicate migrations or
leave half-wired RPC methods on `master`.

### Phase 2 — local-mode runtime + reviews + tail + notifier

- **2a — persistence.** `SqliteStore`, `EventBus` with persisted-
  cursor replay + live broadcast, lease-based task queue with
  three-scope concurrency caps, Appendix A migrations, startup
  lease reaper.
- **2b — real runners + worktree + cost.** `ClaudeRunnerAdapter`
  shelling out to the `claude` CLI; `AnthropicRunnerAdapter` over
  the REST API with cost rollup per stage; `WorktreeManager` for
  per-job `git worktree` checkouts; heartbeat / lease renewal.
- **2c — reviews + tail + notifier.** `Review` row + the four
  `*_review` RPC methods; `codeless tail <job-id>` replaying
  persisted events and following live; HMAC-SHA256-signed
  `WebhookNotifier` for `JobFailed` / `ReviewRequested`.

### Phase 3 — hosted server, HTTP/SSE client, CLI parity

- **3a — codeless-server.** axum library exposing every `RpcServer`
  method as `POST /rpc/<method>` plus `GET /events` SSE.
- **3b — codeless-client.** iOS/Android-safe `HttpRpcClient`
  implementing `RpcServer` over reqwest + a hand-rolled SSE parser.
  Mirror of `routes::map_err` so error variants don't drift.
- **3c-e — CLI dual-mode.** `--core URL --token T` global flags;
  `codeless tail`, `codeless jobs`, `codeless review`, and the
  shared `rpc_open::build_dual_mode` dispatcher pick `HttpRpcClient`
  or `InProcessRpc` accordingly.
- **3f — codeless cost summary.** Client-side rollup over
  `Job.cost_cents` (total + by-status + by-runner).
- **3g — `/healthz`, `/version`, `repos add/remove`.** Unauthenticated
  probes; CRUD CLI verbs.
- **3h — server tracing.** `tower-http::TraceLayer` + `tracing-
  subscriber` (env-filter, fmt to stderr) wired in.
- **3i — `--json` output.** `repos list`, `jobs list`, `cost summary`
  all accept `--json` for shell-pipeline consumers.
- **3j — server background driver.** `spawn_job_driver_loop`:
  subscribes to `JobQueued`, runs `drive_job` per job, Semaphore
  caps concurrency. Hosted-mode jobs now reach `Completed` without
  anyone running `codeless run`.
- **3k — worktree provisioning.** `--worktree-root` flag; each job
  gets `<root>/job-<id>` on a `codeless/job-<id>` branch.
- **3l — anthropic + claude factory.** `--enable-claude` /
  `--enable-anthropic` opt-in; the latter reads `anthropic_api_key`
  from the secrets file.
- **3m — webhook notifier.** When `notifier_webhook_url` +
  `notifier_webhook_hmac_key_hex` are both set in the secrets file,
  the runtime fires HMAC-signed POSTs on `JobFailed` /
  `ReviewRequested`.

## Crate layout (R1 — enforceable)

| crate                       | mobile-safe | new in this PR |
|-----------------------------|-------------|----------------|
| `codeless-types`            | yes         | (already)      |
| `codeless-rpc`              | yes         | review methods |
| `codeless-runtime`          | no          | driver loop, notifier |
| `codeless-adapters-host`    | no          | worktree, secrets |
| `codeless-server`           | no          | new (axum)     |
| `codeless-client`           | yes         | new (HttpRpcClient) |
| `codeless-cli`              | no          | all the verbs  |
| `codeless-tauri-desktop`    | no          | (stub)         |

R1 is enforceable by `cargo check`: `codeless-client` does not pull
`tokio::process`, `std::process`, or any host-only crate. Grep
confirms (the only `process::Command` is in `codeless-adapters-host`
+ `codeless-runtime::driver` worktree path).

## What's not in this PR

- The Tauri desktop shell (Phase 5).
- The mobile shells (Phase 6).
- UI work — `ui/codeless-ui/` changes (shell-injection adapters,
  `fs.*` / `secrets.*` RPC wiring) live on this same branch but are
  driven by a parallel session and out of scope here. Land them
  together if reviewing the branch as a whole.

## Verify

```sh
cargo test --workspace                                  # 157 passed
cargo clippy --workspace --all-targets -- -D warnings   # clean
cargo fmt --check                                       # clean
```

Manual dry run (Phase 3a) captured in
`DOCS/sessions/2026-05-12-phase-3a-terax-demo.md` Notes. Demo
quickstart in `DEMO-UI.md`.

## Test plan

- [ ] `cargo test --workspace` green on a clean checkout.
- [ ] `codeless serve --init-token` prints a 32-char hex string and
      persists it to `core_bearer_token`.
- [ ] `codeless serve` binds; `GET /healthz` returns `ok` without
      a token; `POST /rpc/list_repos` returns 401 without bearer,
      200 with the right one.
- [ ] Submit a `mock` job via HTTP against a fresh `--db`; without
      anyone running `codeless run`, the job reaches `Completed`
      within seconds.
- [ ] `codeless --core URL --token T repos list --json` returns a
      JSON array; the same against `jobs list` and `cost summary`.
- [ ] `codeless tail --core URL --token T <job-id>` replays SSE
      events and exits on the terminal frame.
- [ ] Configure `notifier_webhook_url` + `notifier_webhook_hmac_key_hex`
      against a wiremock; submit a `FAIL`-prompt mock job; observe
      the signed POST on the receiver.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
