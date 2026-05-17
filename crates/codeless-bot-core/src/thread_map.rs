//! In-memory `(channel, thread_ts) -> JobId` cache. Each outbound
//! failure notification posts as a top-level message and records its
//! `ts` here; replies to that thread arrive with a populated
//! `thread_ts` that the dispatcher resolves to the job id without
//! the operator having to retype it.
//!
//! The map is intentionally lossy: the process restarts, the map
//! empties, and old failure threads stop accepting bare-verb replies.
//! That is acceptable because the SCOPE doc nails the load-bearing
//! invariant in a different place — the *parser*. Cold messages need
//! an explicit id; only thread replies elide it. So a missing entry
//! degrades to the same error path as `resume` in a random channel:
//! `MissingJobId`, with a help reply pointing at the cold grammar.
//! No silent wrong-job dispatch is possible.
//!
//! Stage 4 wires the lookup half. Stage 6 (outbound notifications)
//! is the only writer — `record` is exposed here for that stage to
//! call when posting a failure thread, but the dispatcher itself
//! never writes to the map. The split keeps the seam clean even
//! though the writer's call site does not exist yet.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use codeless_types::JobId;

/// Shared thread-context cache. Cloneable handles point at one
/// underlying `RwLock<HashMap>`; the lock is held briefly per access
/// because the map is tiny (one entry per outbound failure post,
/// bounded by the operator's actual failure rate). The reader path
/// (`lookup`) is what the dispatcher hits per inbound message; the
/// writer path is exercised once per outbound post in stage 6.
#[derive(Debug, Clone, Default)]
pub struct ThreadMap {
    inner: Arc<RwLock<HashMap<ThreadKey, JobId>>>,
}

/// Composite key. Slack `thread_ts` strings are not globally unique —
/// the same value can occur in different channels — so the channel id
/// is part of the key. Using a tuple of owned strings avoids
/// lifetime-juggling in the dispatch path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ThreadKey {
    channel: String,
    thread_ts: String,
}

impl ThreadMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or overwrite) a thread->job mapping. Idempotent on the
    /// same key — a stage 6 retry that posts the same notification
    /// twice would not double-book a thread.
    pub fn record(&self, channel: &str, thread_ts: &str, job_id: JobId) {
        let mut guard = self
            .inner
            .write()
            .expect("ThreadMap lock poisoned; the runtime is shutting down");
        guard.insert(
            ThreadKey {
                channel: channel.to_string(),
                thread_ts: thread_ts.to_string(),
            },
            job_id,
        );
    }

    /// Look up the job id bound to a thread. `None` for any of the
    /// three "no thread context" cases: the message was top-level
    /// (no `thread_ts`), the thread is one the bot did not post, or
    /// the process was restarted and the in-memory map was lost.
    pub fn lookup(&self, channel: &str, thread_ts: &str) -> Option<JobId> {
        let guard = self
            .inner
            .read()
            .expect("ThreadMap lock poisoned; the runtime is shutting down");
        guard
            .get(&ThreadKey {
                channel: channel.to_string(),
                thread_ts: thread_ts.to_string(),
            })
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_returns_none() {
        let map = ThreadMap::new();
        assert!(map.lookup("C1", "1.0").is_none());
    }

    #[test]
    fn record_then_lookup_round_trips() {
        let map = ThreadMap::new();
        let job = JobId::new();
        map.record("C123", "1700000000.000001", job);
        assert_eq!(
            map.lookup("C123", "1700000000.000001"),
            Some(job),
            "lookup must match the recorded job id"
        );
    }

    #[test]
    fn channel_is_part_of_the_key() {
        // Slack thread timestamps are not globally unique; the
        // channel id is what disambiguates them.
        let map = ThreadMap::new();
        let job_a = JobId::new();
        let job_b = JobId::new();
        map.record("C1", "1.0", job_a);
        map.record("C2", "1.0", job_b);
        assert_eq!(map.lookup("C1", "1.0"), Some(job_a));
        assert_eq!(map.lookup("C2", "1.0"), Some(job_b));
    }

    #[test]
    fn cloned_handle_sees_same_map() {
        let map = ThreadMap::new();
        let map2 = map.clone();
        let job = JobId::new();
        map.record("C1", "1.0", job);
        assert_eq!(map2.lookup("C1", "1.0"), Some(job));
    }
}
