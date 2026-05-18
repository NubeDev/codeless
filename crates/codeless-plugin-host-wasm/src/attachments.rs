//! Host implementation of the `codeless:attachments/store@0.1.0` WIT
//! interface (see `crates/codeless-tool-wit/wit/attachments.wit`).
//!
//! The store is a trait so the production runtime can plug a real
//! `assistant_attachments` writer in (substrate item 7) while
//! integration tests use the in-memory implementation in this
//! module. The trait is `async` because the production writer goes
//! through `sqlx` and uploads bytes to the on-disk attachment dir;
//! all calls cross `tokio::task` boundaries.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

/// Structured failure shape mirroring the WIT `attachment-error`
/// variant. Carried at the trait level so the host implementation
/// can return a typed error; the WIT bindings translate this into
/// the `attachment-error` discriminant the guest sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentError {
    /// Capability not granted. The linker gate should normally
    /// prevent the call from being reachable in the first place;
    /// this is a defence-in-depth return for paths that bypass the
    /// linker (e.g. a future inline-instantiation that hands the
    /// interface unconditionally).
    Denied,
    /// Id well-formed but no row found in the thread's scope.
    NotFound,
    /// Id failed structural validation.
    InvalidId(String),
    /// Host-side IO failure. The carried string is operator-facing.
    Io(String),
}

/// Host-side surface for the WIT `store` interface. One impl per
/// codeless deployment; the in-memory [`InMemoryAttachmentStore`] is
/// what the test harness uses, and a `SqliteAttachmentStore` (or the
/// existing `assistant_attachments` writer) plugs in the same shape.
#[async_trait]
pub trait AttachmentStore: Send + Sync + 'static {
    /// Look up a previously-minted attachment by id and return its
    /// bytes. `thread_id` is the call-scope from `tool-call.thread-id`
    /// so the store can refuse cross-thread access without the
    /// plugin needing to plumb scopes manually.
    async fn read(&self, thread_id: &str, id: &str) -> Result<Vec<u8>, AttachmentError>;

    /// Persist a new attachment under the given thread and return
    /// its id.
    async fn mint(
        &self,
        thread_id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<String, AttachmentError>;
}

/// In-memory store keyed by `(thread_id, attachment_id)`. Used by
/// `plugin_wasm_e2e::wasm_plugin_attachment_round_trip`; the
/// production server wires the `assistant_attachments` table-backed
/// implementation in stage 13's manifest -> registry hookup.
pub struct InMemoryAttachmentStore {
    rows: Mutex<HashMap<(String, String), StoredAttachment>>,
    next_id: Mutex<u64>,
}

#[derive(Debug, Clone)]
struct StoredAttachment {
    #[allow(dead_code)]
    filename: String,
    bytes: Vec<u8>,
}

impl InMemoryAttachmentStore {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
        }
    }
}

impl Default for InMemoryAttachmentStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AttachmentStore for InMemoryAttachmentStore {
    async fn read(&self, thread_id: &str, id: &str) -> Result<Vec<u8>, AttachmentError> {
        let rows = self
            .rows
            .lock()
            .map_err(|e| AttachmentError::Io(format!("store mutex poisoned: {e}")))?;
        match rows.get(&(thread_id.to_string(), id.to_string())) {
            Some(row) => Ok(row.bytes.clone()),
            None => Err(AttachmentError::NotFound),
        }
    }

    async fn mint(
        &self,
        thread_id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<String, AttachmentError> {
        let mut next = self
            .next_id
            .lock()
            .map_err(|e| AttachmentError::Io(format!("id mutex poisoned: {e}")))?;
        *next += 1;
        let id = format!("att-{}", *next);
        drop(next);
        let mut rows = self
            .rows
            .lock()
            .map_err(|e| AttachmentError::Io(format!("store mutex poisoned: {e}")))?;
        rows.insert(
            (thread_id.to_string(), id.clone()),
            StoredAttachment {
                filename: filename.to_string(),
                bytes: bytes.to_vec(),
            },
        );
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_round_trips_per_thread() {
        let store = InMemoryAttachmentStore::new();
        let id = store
            .mint("t1", "note.md", b"# hi\n")
            .await
            .expect("mint succeeds with default-allow store");
        let back = store.read("t1", &id).await.expect("read same thread");
        assert_eq!(back, b"# hi\n");
        // Cross-thread lookup must miss; the store keys on
        // `(thread_id, id)` so a plugin handing another thread's id
        // back cannot resolve it.
        let miss = store.read("t2", &id).await.unwrap_err();
        assert!(matches!(miss, AttachmentError::NotFound));
    }
}
