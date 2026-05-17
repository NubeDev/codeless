# codeless-tauri-desktop

Tauri 2 desktop shell for Codeless. Launches the React UI as a native
window backed by an embedded `codeless-runtime` — no loopback HTTP,
no second process, no bearer-token auth.

## How it works

Every method on the `RpcServer` trait is exposed as a `#[tauri::command]`
shim (`rpc_<method>`). The UI's `TauriIpcClient` calls these via
`invoke()`. Streaming subscriptions use Tauri 2's typed
`Channel<EventEnvelope>` with a per-channel cancellation token managed
by `rpc_subscribe` / `rpc_unsubscribe`.

The wire contract is locked by
[`ui/codeless-ui/src/lib/rpc/tauri-ipc-client.ts`](../../ui/codeless-ui/src/lib/rpc/tauri-ipc-client.ts).

## Prerequisites

### System libraries (Linux)

```sh
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libsoup-3.0-dev libdbus-1-dev \
  libappindicator3-dev librsvg2-dev patchelf pkg-config libssl-dev
```

### macOS

Xcode Command Line Tools (`xcode-select --install`).

### Tauri CLI

```sh
cargo install tauri-cli --version "^2"
```

## Development

```sh
# From the workspace root (codeless/)
cargo tauri dev -p codeless-tauri-desktop
```

This starts the Vite dev server for the UI and opens the native window
with hot-reload.

## Production build

```sh
cargo tauri build -p codeless-tauri-desktop
```

The bundled app lands in `target/release/bundle/`.

## Source layout

```
src/
  main.rs           — boot: InProcessRpc + driver + stage recorder + Tauri builder
  state.rs          — AppState, SubscriptionMap
  boot.rs           — runtime wiring (SQLite, HostFs, worktrees, agent chat)
  error.rs          — CommandError (Serialize wrapper around RpcError)
  commands/
    mod.rs           — generate_handler![...] registration
    server_info.rs   — rpc_server_info
    subscribe.rs     — rpc_subscribe + rpc_unsubscribe + forwarder task
    repos.rs         — rpc_add_repo, rpc_remove_repo, rpc_list_repos
    jobs.rs          — job lifecycle (~23 commands)
    fs.rs            — rpc_fs_* (~9 commands)
    reviews.rs       — review gate commands (~4)
    assistant.rs     — assistant thread/message commands (~8)
    chat.rs          — rpc_agent_chat, rpc_cancel_chat_task, rpc_stop_active
    personas.rs      — persona CRUD (~4)
    scope_patches.rs — scope patch commands + rpc_set_job_policy (~6)
    workspaces.rs    — workspace attach/detach/list/validate (~4)
```
