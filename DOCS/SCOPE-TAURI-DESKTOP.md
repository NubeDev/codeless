# SCOPE-TAURI-DESKTOP

Phase 5. Wire `codeless-tauri-desktop` so the React UI in
`ui/codeless-ui/` launches as a native window backed by an embedded
`codeless-runtime` — no `axum`, no loopback port, no second process.

Read first:
- [`SCOPE.md`](./SCOPE.md) — crate layout (load-bearing) and the four
  shells the same UI ships into.
- [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md) — why the UI imports
  `RpcClient` only.
- [`../ui/codeless-ui/src/lib/rpc/tauri-ipc-client.ts`](../ui/codeless-ui/src/lib/rpc/tauri-ipc-client.ts)
  — the wire contract the Rust side must satisfy. **It is the spec.**

If anything below contradicts `tauri-ipc-client.ts`, the TS file
wins — fix this doc.

## Thesis

The browser shell already works against `codeless-server` over HTTP +
SSE. The desktop shell exists so a single-user "open Codeless, drive
jobs against my repos" path doesn't need a separate background
service, port allocation, or token plumbing — the runtime lives in
the same process as the webview and the wire is Tauri IPC instead of
loopback HTTP. Everything else (UI, runtime semantics, SQLite shape,
event bus) is identical to the server path. The desktop crate is a
**second transport adapter** in front of the same
[`InProcessRpc`](../crates/codeless-runtime/src/rpc/mod.rs), not a
fork of anything.

## Non-goals

- No new RPC methods. The Tauri commands are a 1:1 projection of
  [`RpcServer`](../crates/codeless-rpc/src/server.rs).
- No desktop-only UI screens. The desktop window mounts the same
  React entry as the browser shell, differing only by which
  `RpcClient` the [`src/shells/desktop/main.tsx`](../ui/codeless-ui/src/shells/desktop/main.tsx)
  entry constructs.
- No mobile in this phase. Mobile (Phase 6) compiles `codeless-types`
  + `codeless-client` only and talks to a hosted core over SSE.
- No multi-user / hosted mode. The desktop binary is single-user,
  same-host-same-uid, with the SQLite file under the OS data dir.
- No `codeless-adapters-desktop` crate. Per the SCOPE.md table, it
  only gets created when there is more than one thing to put in it;
  until then, anything desktop-flavoured lives inside this crate.

## Wire contract (locked by the TS side)

Every TS call from `TauriIpcClient` maps to exactly one Tauri command.
The Rust side hand-rolls a `#[tauri::command]` shim per `RpcServer`
method — there are ~60 of them. This is the boilerplate cost of not
having `tauri-specta` codegen wired up in Phase 5; if the boilerplate
becomes painful, a follow-up can macro it from a walk of the trait.

| TS call site | Tauri command | Rust signature |
|---|---|---|
| `client.call(method, args)` | `rpc_<method>` | `async fn rpc_<method>(state: State<AppState>, args: <ArgsStruct>) -> Result<<ResultStruct>, RpcError>` |
| `client.serverInfo()` | `rpc_server_info` | `async fn rpc_server_info(state: State<AppState>) -> Result<ServerInfo, RpcError>` |
| `client.subscribe(filter, since)` | `rpc_subscribe` | `async fn rpc_subscribe(state: State<AppState>, args: SubscribeArgs, channel: tauri::ipc::Channel<EventEnvelope>) -> Result<(), RpcError>` |
| (auto on `return()` / `AbortController`) | `rpc_unsubscribe` | `async fn rpc_unsubscribe(state: State<AppState>, channel_id: u32) -> Result<(), RpcError>` |

Arg envelope: every call from the TS side is shaped `{ args: <T> }`
so the Rust command takes a single typed parameter — see
[`tauri-ipc-client.ts`](../ui/codeless-ui/src/lib/rpc/tauri-ipc-client.ts)
lines 25–47. `rpc_server_info` is the one exception (no args).

`SubscribeArgs` shape:
```rust
struct SubscribeArgs { filter: EventFilter, since: Option<Since> }
```
Matches the TS-side `{ filter, since: since ?? null }`.

## State held by the desktop shell

```rust
struct AppState {
    rpc: Arc<InProcessRpc>,
    server_info: Arc<ServerInfo>,
    subs: Arc<SubscriptionMap>,
}

type SubscriptionMap = parking_lot::Mutex<HashMap<u32, CancellationToken>>;
```

- `rpc` — same `Arc<InProcessRpc>` the hosted server holds in
  `codeless-server::AppState`. Constructed once on boot.
- `server_info` — analogue of `codeless-server::AppState.server_info`.
  The desktop side fills it with a desktop-flavoured `ServerInfo`
  (build version, available runners discovered from the host PATH).
- `subs` — `Channel::id() -> CancellationToken`. `rpc_subscribe`
  inserts; `rpc_unsubscribe` and the forwarder task's drop-guard
  both remove. This is the analogue of `ChatCancels` inside
  `InProcessRpc`, not a runtime concern — subscription handles only
  exist for the lifetime of the IPC channel.

## Subscribe forwarder

`rpc_subscribe` spawns a `tokio::task` that:

1. Calls `state.rpc.subscribe(args.filter, args.since)` to get an
   `EventStream`.
2. Polls the stream; each `Ok(env)` is sent on the
   `Channel<EventEnvelope>`.
3. Stops on (a) stream end, (b) channel send error (UI gone),
   (c) cancellation token fired by `rpc_unsubscribe`.
4. Drops its entry from `SubscriptionMap` on exit (drop-guard).

`rpc_subscribe` itself returns `Ok(())` as soon as the forwarder is
spawned; the TS side awaits this before yielding from the async
iterator. Errors from the initial `subscribe()` call are reported
via the Promise from `invoke("rpc_subscribe", …)`, matching the TS
`subscribeCall.catch(...)` path on lines 78–82.

## Runtime wiring (boot)

`InProcessRpc` exposes a builder. The desktop boot must call every
builder method or the corresponding RPC surface returns `Internal`
(by design — see the doc comments on each field in
[`crates/codeless-runtime/src/rpc/mod.rs`](../crates/codeless-runtime/src/rpc/mod.rs)).
Minimal viable boot:

```rust
let data_dir = directories::ProjectDirs::from("dev", "codeless", "Codeless")
    .ok_or(BootError::NoDataDir)?
    .data_dir()
    .to_path_buf();
let db_path        = data_dir.join("codeless.sqlite");
let worktree_base  = data_dir.join("worktrees");
let assistant_root = data_dir.join("assistant");
let workspace_root = std::env::current_dir()?; // overridable via UI later

let rpc = InProcessRpc::with_file(&db_path).await?
    .with_fs(Arc::new(HostFs::new(workspace_root.clone())))
    .with_worktrees(Arc::new(WorktreeManager::new(worktree_base)))
    .with_agent_chat(Arc::new(ai_runner::Registry::with_defaults()), workspace_root)
    .with_assistant_data_dir(assistant_root);

let rpc = Arc::new(rpc);
spawn_job_driver_loop_with_retry(Arc::clone(&rpc), /* policy */ default_retry_policy());
spawn_heartbeat(Arc::clone(&rpc.store()), /* interval */ Duration::from_secs(5));
```

Open questions deliberately left for the implementation:

- **Workspace root selection.** The browser shell takes the
  workspace from `attach_workspace` RPC calls; the desktop shell can
  use the same flow once the window is up. Boot-time default is the
  process cwd so the binary works when launched from a terminal.
- **Secrets file location.** Pick the same `data_dir` path the CLI
  uses (`<data_dir>/secrets.json`) so `codeless secrets …` and the
  desktop binary see the same set.

## Cargo layout

```toml
# crates/codeless-tauri-desktop/Cargo.toml
[package]
name = "codeless-tauri-desktop"
# … existing workspace inheritance …

[[bin]]
name = "codeless-tauri-desktop"
path = "src/main.rs"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri              = { version = "2", features = ["macos-private-api"] }
tokio              = { workspace = true, features = ["full"] }
serde              = { workspace = true }
serde_json         = { workspace = true }
parking_lot        = "0.12"
directories        = "5"
tokio-util         = { version = "0.7", features = ["rt"] }
futures-util       = { workspace = true }
tracing            = { workspace = true }
codeless-types     = { path = "../codeless-types" }
codeless-rpc       = { path = "../codeless-rpc" }
codeless-runtime   = { path = "../codeless-runtime" }
codeless-adapters-host = { path = "../codeless-adapters-host" }
ai-runner          = { path = "../../../ai-runner" }
```

Source layout:

```
crates/codeless-tauri-desktop/
├── Cargo.toml
├── build.rs                    # one-liner: tauri_build::build()
├── tauri.conf.json             # window + bundling config
├── capabilities/
│   └── default.json            # opt-in to the IPC + window APIs we use
├── icons/                      # generated by `cargo tauri icon`
└── src/
    ├── main.rs                 # boot + tauri::Builder::default()…run()
    ├── state.rs                # AppState, SubscriptionMap
    ├── boot.rs                 # InProcessRpc wiring (above)
    ├── commands/
    │   ├── mod.rs              # generate_handler![…] registration
    │   ├── server_info.rs      # rpc_server_info
    │   ├── subscribe.rs        # rpc_subscribe + rpc_unsubscribe + forwarder
    │   ├── repos.rs            # rpc_add_repo, rpc_remove_repo, rpc_list_repos
    │   ├── jobs.rs             # rpc_submit_job, rpc_get_job, … (~15)
    │   ├── fs.rs               # rpc_fs_*           (~9)
    │   ├── reviews.rs          # rpc_list_reviews, … (~4)
    │   ├── assistant.rs        # rpc_*_assistant_*  (~9)
    │   ├── chat.rs             # rpc_agent_chat, rpc_cancel_chat_task, …
    │   ├── personas.rs         # rpc_*_persona      (~4)
    │   ├── scope_patches.rs    # rpc_*_scope_patch  (~5)
    │   └── workspaces.rs       # rpc_*_workspace    (~3)
```

The per-file split matches CLAUDE.md R3 — one concept per file. Each
command is a trivial shim:

```rust
#[tauri::command]
async fn rpc_submit_job(
    state: tauri::State<'_, AppState>,
    args: SubmitJobArgs,
) -> Result<Job, RpcError> {
    state.rpc.submit_job(args).await
}
```

## `tauri.conf.json` essentials

- `build.frontendDist`: `"../../ui/codeless-ui/dist"`
- `build.devUrl`: `"http://localhost:5173"` (matches the Vite default)
- `build.beforeDevCommand`: `"pnpm -C ../../ui/codeless-ui dev"`
- `build.beforeBuildCommand`: `"pnpm -C ../../ui/codeless-ui build"`
- `app.windows[0]`: title `"Codeless"`, width `1400`, height `900`,
  `titleBarStyle: "Overlay"` (macOS), `transparent: false`.
- `app.security.csp`: `null` for dev, locked-down value for prod
  (Tauri 2 default starting point is fine).
- `identifier`: `dev.codeless.desktop`.

## Constraints

- **R1 — process spawning.** Tauri itself does not violate the
  `process_spawn` probe; the probe only scans codeless source. The
  shell never calls `std::process::Command` directly — anything that
  needs to spawn (e.g. opening files in the host editor) goes through
  a Tauri plugin (which lives outside our source tree) or through
  `codeless-adapters-host`. Picked up by
  [`crates/codeless-predicates/src/probes/process_spawn.rs`](../crates/codeless-predicates/src/probes/process_spawn.rs).
- **R1 — mobile-safe crates.** This crate must not appear in the dep
  tree of any mobile-safe crate. Mobile (Phase 6) compiles only
  `codeless-types` + `codeless-client`.
- **R2 — comments.** No emojis, no task-status comments, no decorative
  banners. The generated `tauri::generate_handler![…]` call is the
  one place where a per-line list of every command is unavoidable;
  keep it as a flat list, no commentary per line.
- **R5 — tests.** `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check` all stay green.
  Integration test: a headless Tauri app boot that submits a mock
  job and asserts the `job-completed` envelope arrives over the
  Channel. The runner is the existing `MockRunner` harness so the
  test doesn't depend on a real `claude` binary.

## Operational prerequisites (devs and CI)

These are environmental, not code. Document in `README.md` when the
crate lights up.

- `cargo install tauri-cli --version "^2"` for `cargo tauri dev` and
  `cargo tauri build`.
- Linux: `webkit2gtk-4.1`, `libsoup-3.0`, `libappindicator3`,
  `librsvg2`, `patchelf` (per Tauri 2 docs).
- macOS: Xcode CLT.
- Windows: WebView2 runtime (preinstalled on Win11; bundled in the
  installer for Win10).
- MSRV: 1.78. Tauri 2.x current MSRV is satisfied; pin the
  `tauri` minor version explicitly so a later bump doesn't quietly
  raise the workspace MSRV.

## What this does not change

- `codeless-server` continues to exist and serves the browser shell.
- The CLI continues to run `InProcessRpc` directly. Adding the Tauri
  crate does not move any code out of the CLI.
- The HTTP/SSE wire contract is untouched. A user who launches the
  desktop app and a separate browser tab against `codeless serve`
  sees two independent runtimes, each with its own SQLite file,
  unless they point the desktop boot at the same `data_dir` the
  server uses.

## Out of scope, tracked separately

- **Auto-updater.** Tauri 2's updater plugin is the obvious answer
  but needs a signing-key story. Defer to a follow-up.
- **Code signing / notarisation.** Per-OS packaging is its own job.
- **Background mode / system tray.** The first cut is a foreground
  window only.
- **Multiple windows.** The settings window in the Terax-derived UI
  uses Tauri's `WebviewWindow`; bringing it back is a small follow-up
  once the main window is up.
- **`tauri-specta` codegen** to replace the hand-rolled command
  shims. Worth it once the trait shape stabilises.
