//! Codeless runtime — owns the job state machine, the in-memory store
//! (until stage 4 swaps in sqlx), the event bus, and the in-process
//! implementation of `codeless_rpc::RpcServer`. See `DOCS/SCOPE.md`
//! "Crate layout": this crate is host-only and never compiled for the
//! mobile shell; mobile reaches the runtime over the network via
//! `codeless-client`.

pub mod event_bus;
pub mod rpc;
pub mod store;
pub mod time;

pub use event_bus::{EventBus, SubscribeFilter};
pub use rpc::InProcessRpc;
pub use store::MemoryStore;
pub use time::now_ms;
