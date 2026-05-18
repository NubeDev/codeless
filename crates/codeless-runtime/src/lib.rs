//! Codeless runtime — owns the job state machine, the in-memory store
//! (until stage 4 swaps in sqlx), the event bus, and the in-process
//! implementation of `codeless_rpc::RpcServer`. See `DOCS/SCOPE.md`
//! "Crate layout": this crate is host-only and never compiled for the
//! mobile shell; mobile reaches the runtime over the network via
//! `codeless-client`.

pub mod anthropic_runner;
pub mod attached_workspaces;
pub mod auto_bypass_guard;
pub mod auto_bypass_policy;
pub mod auto_bypass_presets;
pub mod claude_runner;
pub mod codex_runner;
pub mod copilot_runner;
pub mod diff_verify;
pub mod driver;
pub mod event_bus;
pub mod handover;
pub mod heartbeat;
pub mod job_dir;
pub mod job_driver_loop;
pub mod migrations;
pub mod mock_runner;
pub mod notifier;
pub mod queue_config;
pub mod review_gate;
pub mod rpc;
pub mod rule_bearing_files;
pub mod runner;
pub mod scope_patch_emit;
pub mod scope_patch_queue;
pub mod session_idle;
pub mod session_log;
pub mod stage_recorder;
pub mod state_machine;
pub mod store;
pub mod template;
pub mod template_runner;
pub mod time;
pub mod tracing_init;
pub mod verify_runner;
pub mod webhook;
pub mod workspace_liveness;

pub use anthropic_runner::AnthropicRunnerAdapter;
pub use claude_runner::{parse_permission_mode, ClaudeRunnerAdapter};
pub use codex_runner::CodexRunnerAdapter;
pub use copilot_runner::CopilotRunnerAdapter;
pub use driver::drive_job;
pub use event_bus::{EventBus, SubscribeFilter};
pub use heartbeat::spawn_heartbeat;
pub use job_driver_loop::{
    spawn_job_driver_loop, spawn_job_driver_loop_with_retry, DriverLoopHandle, RetryPolicy,
    RunnerFactory,
};
pub use migrations::MIGRATOR;
pub use mock_runner::{MockRunner, MockStep};
pub use notifier::{
    spawn_notifier, NotificationKind, NotificationPayload, Notifier, NotifierError,
};
pub use queue_config::QueueConfig;
pub use rpc::InProcessRpc;
pub use rpc::{ChatCancelEntry, ChatCancels};
pub use runner::{Runner, RunnerContext, RunnerOutcome};
pub use session_idle::{
    resolve_stage_resume, spawn_idle_sweeper, sweep_once, ResumeDecision,
    DEFAULT_SESSION_IDLE_TIMEOUT,
};
pub use stage_recorder::spawn_stage_recorder;
pub use state_machine::{
    is_terminal_job, transition_job, transition_stage, transition_task, TransitionError,
};
pub use store::SqliteStore;
pub use time::now_ms;
pub use tracing_init::{try_init_json, try_init_pretty};
pub use webhook::{WebhookConfig, WebhookNotifier, WebhookSetupError};
pub use workspace_liveness::{spawn_workspace_liveness_sweep, WORKSPACE_LIVENESS_PERIOD};
