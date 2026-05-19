// Tauri 2 desktop shell. Embeds an `InProcessRpc` runtime and exposes
// every `RpcServer` method as a `#[tauri::command]` — no second
// process for IPC. Alongside the IPC surface, the desktop also binds
// `codeless-server` on an ephemeral loopback port so external tools
// (scripts, AI agents driving the runtime over HTTP) reach the same
// in-process runtime without launching a sidecar `codeless serve`.
// The bound URL is surfaced to the UI through `ServerInfo.rest_url`
// so the settings panel can show what to point external tools at.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod boot;
mod commands;
mod error;
mod state;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::oneshot;

use codeless_rpc::ServerInfo;
use codeless_runtime::{spawn_job_driver_loop, spawn_stage_recorder};
use codeless_server::{serve_with_shutdown, AppState as RestAppState};

use state::AppState;

/// Resolve the workspace this launch is scoped to. The desktop binary
/// owns exactly one workspace per launch (see `boot::boot`); a second
/// launch on a different folder gets its own SQLite + worktrees +
/// event bus under a different slug. Precedence:
///   1. `--workspace <path>` argv flag
///   2. `CODELESS_WORKSPACE` env var
///   3. process cwd
///
/// The picker UI surface that would let the desktop start without a
/// pre-selected workspace and then route the user into one is a
/// follow-up; until it lands, cwd is the implicit default and matches
/// how the binary is launched today.
fn resolve_workspace_arg() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--workspace=") {
            return PathBuf::from(value);
        }
        if arg == "--workspace" {
            if let Some(value) = args.next() {
                return PathBuf::from(value);
            }
        }
    }
    if let Ok(value) = std::env::var("CODELESS_WORKSPACE") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() {
    let workspace = resolve_workspace_arg();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let boot::BootResult {
                    rpc,
                    server_info,
                    runner_factory,
                } = boot::boot(workspace).await.expect("runtime boot failed");

                let _stage_recorder = spawn_stage_recorder(rpc.bus().clone(), rpc.store().clone())
                    .await
                    .expect("stage recorder failed to start");

                let _driver = spawn_job_driver_loop(rpc.clone(), runner_factory, None, 4)
                    .await
                    .expect("driver loop failed to start");

                // Bind the REST server before publishing AppState so the
                // UI sees `rest_url` populated on its first
                // `serverInfo()` call. The oneshot completes from inside
                // `serve_with_shutdown`'s `on_bound` callback, which
                // fires the instant the listener is up — typically
                // sub-millisecond. The `rest_addr` task continues to own
                // the server future and lives for the lifetime of the
                // window.
                let (bound_tx, bound_rx) = oneshot::channel::<SocketAddr>();
                let server_info_for_rest: ServerInfo = (*server_info).clone();
                let rpc_for_rest: Arc<dyn codeless_rpc::RpcServer> = rpc.clone();
                tauri::async_runtime::spawn(async move {
                    let state =
                        RestAppState::open(rpc_for_rest).with_server_info(server_info_for_rest);
                    let mut tx = Some(bound_tx);
                    let bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
                    if let Err(e) = serve_with_shutdown(bind_addr, state, move |addr| {
                        if let Some(tx) = tx.take() {
                            let _ = tx.send(addr);
                        }
                        eprintln!("codeless-desktop REST listening on http://{addr}");
                    })
                    .await
                    {
                        eprintln!("codeless-desktop REST server exited: {e}");
                    }
                });

                let bound_addr = bound_rx.await.expect("REST server failed to bind");
                let mut info_with_url: ServerInfo = (*server_info).clone();
                info_with_url.rest_url = Some(format!("http://{bound_addr}"));
                let server_info = Arc::new(info_with_url);

                let app_state = AppState {
                    rpc,
                    server_info,
                    subs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                };
                handle.manage(app_state);
            });
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("error running Codeless desktop");
}
