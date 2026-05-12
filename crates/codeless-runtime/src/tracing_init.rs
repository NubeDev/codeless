use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::TryInitError;
use tracing_subscriber::EnvFilter;

/// Default filter applied when `RUST_LOG` is unset. Chosen so a quiet
/// shell still sees `info`-level job/stage/task transitions but is not
/// flooded by debug logs from `sqlx`, `hyper`, or `tokio` internals.
const DEFAULT_FILTER: &str = "info,sqlx=warn,hyper=warn";

/// Install a JSON-to-stdout `tracing` subscriber as the process-global
/// default. Honours `RUST_LOG`; falls back to a quiet info-level
/// filter that lets through job/stage/task spans but mutes noisy
/// downstream crates (see `DEFAULT_FILTER`).
///
/// Idempotency: returns `TryInitError` if a subscriber is already set
/// — callers in tests deliberately ignore that error so multiple tests
/// in the same process do not race. The CLI / server callers in later
/// stages will treat it as fatal on first start.
pub fn try_init_json() -> Result<(), TryInitError> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .json()
                .with_target(true)
                .with_current_span(true)
                .with_span_list(false),
        )
        .try_init()
}

/// Pretty-printed dev variant. Hosted mode picks JSON
/// (SCOPE.md "Tracing baseline: JSON to stdout in hosted mode, pretty
/// in dev"); CLI invocations in a terminal prefer this.
pub fn try_init_pretty() -> Result<(), TryInitError> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .try_init()
}
