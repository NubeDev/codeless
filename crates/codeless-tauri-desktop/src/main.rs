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
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::oneshot;

use codeless_rpc::ServerInfo;
use codeless_runtime::{spawn_job_driver_loop, spawn_stage_recorder};
use codeless_server::{AppState as RestAppState, serve_with_shutdown};

use state::AppState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let boot::BootResult {
                    rpc,
                    server_info,
                    runner_factory,
                } = boot::boot().await.expect("runtime boot failed");

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
                    let state = RestAppState::open(rpc_for_rest)
                        .with_server_info(server_info_for_rest);
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
