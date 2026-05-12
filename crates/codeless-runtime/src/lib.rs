//! Codeless runtime — owns the job state machine, the in-memory store
//! (until stage 4 swaps in sqlx), the event bus, and the in-process
//! implementation of `codeless_rpc::RpcServer`. See `DOCS/SCOPE.md`
//! "Crate layout": this crate is host-only and never compiled for the
//! mobile shell; mobile reaches the runtime over the network via
//! `codeless-client`.

pub mod driver;
pub mod event_bus;
pub mod migrations;
pub mod mock_runner;
pub mod rpc;
pub mod runner;
pub mod state_machine;
pub mod store;
pub mod time;
pub mod tracing_init;

pub use driver::drive_job;
pub use event_bus::{EventBus, SubscribeFilter};
pub use migrations::MIGRATOR;
pub use mock_runner::{MockRunner, MockStep};
pub use rpc::InProcessRpc;
pub use runner::{Runner, RunnerContext, RunnerOutcome};
pub use state_machine::{
    is_terminal_job, transition_job, transition_stage, transition_task, TransitionError,
};
pub use store::SqliteStore;
pub use time::now_ms;
pub use tracing_init::{try_init_json, try_init_pretty};
