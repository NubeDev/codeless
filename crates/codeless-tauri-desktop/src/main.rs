// Tauri 2 desktop shell. Embeds an `InProcessRpc` runtime and exposes
// every `RpcServer` method as a `#[tauri::command]` — no loopback
// port, no second process, no HTTP auth.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod boot;
mod commands;
mod error;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use tauri::Manager;

use codeless_runtime::{spawn_job_driver_loop, spawn_stage_recorder};

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
