//! Per-chat short-alias cache. When the bot posts a numbered job
//! list (`status`), it records the `1..N` → `JobId` mapping here so
//! the operator can type `status 3` or `resume 2 bypass` instead of
//! copy-pasting a full 26-character ULID on a phone keyboard.
//!
//! The map is volatile and per-chat: a `status` in chat A does not
//! affect aliases visible in chat B. Each new `status` reply
//! overwrites the previous aliases for that chat — the numbers
//! always reflect the most recent listing the operator saw.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use codeless_types::JobId;

/// Shared alias cache. Cheap to clone (one `Arc`).
#[derive(Debug, Clone, Default)]
pub struct AliasMap {
    inner: Arc<RwLock<HashMap<String, Vec<JobId>>>>,
}

impl AliasMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the alias list for `chat` with a fresh ordered vector
    /// of job IDs (index 0 = alias "1", index 1 = alias "2", …).
    pub fn set(&self, chat: &str, ids: Vec<JobId>) {
        let mut guard = self.inner.write().expect("AliasMap lock poisoned");
        guard.insert(chat.to_string(), ids);
    }

    /// Resolve a 1-based numeric alias in `chat`. Returns `None` when
    /// the alias is out of range or no listing has been posted in
    /// that chat yet.
    pub fn resolve(&self, chat: &str, alias: usize) -> Option<JobId> {
        if alias == 0 {
            return None;
        }
        let guard = self.inner.read().expect("AliasMap lock poisoned");
        guard.get(chat).and_then(|ids| ids.get(alias - 1)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn id(s: &str) -> JobId {
        JobId::from_str(s).unwrap()
    }

    #[test]
    fn set_and_resolve() {
        let map = AliasMap::new();
        let a = id("01KRSGS50WH0HBKX14C04ZW110");
        let b = id("01KRSGS50WH0HBKX14C04ZW111");
        map.set("chat-1", vec![a, b]);
        assert_eq!(map.resolve("chat-1", 1), Some(a));
        assert_eq!(map.resolve("chat-1", 2), Some(b));
        assert_eq!(map.resolve("chat-1", 3), None);
        assert_eq!(map.resolve("chat-1", 0), None);
        assert_eq!(map.resolve("other-chat", 1), None);
    }

    #[test]
    fn overwrite_replaces_previous() {
        let map = AliasMap::new();
        let a = id("01KRSGS50WH0HBKX14C04ZW110");
        let b = id("01KRSGS50WH0HBKX14C04ZW111");
        map.set("c", vec![a]);
        assert_eq!(map.resolve("c", 1), Some(a));
        map.set("c", vec![b]);
        assert_eq!(map.resolve("c", 1), Some(b));
        assert_eq!(map.resolve("c", 2), None);
    }
}
