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

use codeless_runtime::{
    spawn_job_driver_loop, spawn_stage_recorder, MockRunner, MockStep, Runner, RunnerFactory,
    RunnerOutcome,
};
use codeless_types::Job;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let (rpc, server_info) = boot::boot().await.expect("runtime boot failed");

                let _stage_recorder = spawn_stage_recorder(rpc.bus().clone(), rpc.store().clone())
                    .await
                    .expect("stage recorder failed to start");

                let factory = Arc::new(DesktopRunnerFactory);
                let _driver = spawn_job_driver_loop(rpc.clone(), factory, None, 4)
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

/// Minimal runner factory for the desktop shell. Supports only the
/// `mock` runner so the app boots without external dependencies; real
/// runners (`claude`, `codex`, etc.) will be enabled via a settings
/// surface in a follow-up.
struct DesktopRunnerFactory;

impl RunnerFactory for DesktopRunnerFactory {
    fn build(&self, job: &Job) -> Option<Arc<dyn Runner>> {
        if job.runner == "mock" {
            Some(Arc::new(MockRunner::new(vec![MockStep::Finish(
                RunnerOutcome::Completed,
            )])))
        } else {
            None
        }
    }
}
