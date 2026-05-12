use std::time::{SystemTime, UNIX_EPOCH};

use codeless_types::UnixMillis;

/// Wall-clock helper. Lives behind a free function so the runtime can
/// move to an injected clock when deterministic tests need one — the
/// MockRunner harness in stage 5 is the first plausible caller.
pub fn now_ms() -> UnixMillis {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    UnixMillis(d.as_millis().min(i64::MAX as u128) as i64)
}
