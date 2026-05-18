//! Glue between the runtime `EventBus` and the `PlanEngine`.
//!
//! The engine lives in `codeless-tools::plan` (host-agnostic, no
//! tokio runtime handle, no event-bus dependency). The runtime owns
//! the bus, so the subscription loop has to live here — the engine
//! sees envelopes one at a time via `handle_event`. P1 wiring: one
//! engine per process, constructed at boot, subscribed once with no
//! replay. Restart drops in-flight runs alongside the in-memory
//! state, matching the documented P1 scope.

use std::sync::Arc;

use codeless_tools::plan::PlanEngine;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::event_bus::{EventBus, SubscribeFilter};

/// Spawn the long-running task that forwards every envelope on the
/// bus into `PlanEngine::handle_event`. The engine itself filters out
/// non-terminal events and untracked job ids; we deliberately pass
/// the full stream rather than pre-filtering here, so the engine
/// remains the only place that defines what "terminal" means.
pub fn spawn_plan_engine_subscriber(bus: Arc<EventBus>, engine: Arc<PlanEngine>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut stream = match bus.subscribe_since(SubscribeFilter::All, None).await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "plan engine: failed to subscribe to event bus");
                return;
            }
        };
        while let Some(item) = stream.next().await {
            match item {
                Ok(env) => engine.handle_event(&env).await,
                Err(err) => {
                    tracing::warn!(error = %err, "plan engine: event bus stream error");
                }
            }
        }
        tracing::info!("plan engine: event bus stream closed");
    })
}
