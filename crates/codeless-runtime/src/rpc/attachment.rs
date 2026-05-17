//! PS7 -- tool-result attachment reconciliation seam.
//!
//! The runtime's single entry point for taking a tool call's JSON
//! output, finding any attachments the tool's output schema declared
//! via the `{"$ref": "codeless://attachment"}` marker, and turning
//! them into a wire-typed [`AssistantAttachmentCard`] keyed against
//! the live store rows. PS8 (the Assistant agent loop) will call this
//! once per resolved tool result and stash the returned card on the
//! trailing `Tool`-role message's `meta_json`, where the renderer
//! dispatches on [`AssistantAttachmentCard::META_KIND`].
//!
//! Kept narrow on purpose: it does no MCP work, no token streaming,
//! no LLM call. The agent loop owns those; this is the one place the
//! substrate's attachment contract (DOCS/PLUGIN-SUBSTRATE.md item 7)
//! is enforced -- the stored row wins for `filename`/`mime`/`size`,
//! cross-thread ids are rejected, dangling ids are rejected. Doing it
//! in one place means PS8 cannot accidentally skip the
//! reconciliation: the only path to an attachment card is through
//! this function.

// PS8 (the Assistant agent loop) is the only production caller. Until
// PS8 lands the lib build sees no caller outside this module's unit
// tests; suppressing dead_code here keeps the seam shippable now so
// PS8 is a wiring change, not a parallel design pass.
#![allow(dead_code)]

use codeless_rpc::{RpcError, RpcResult};
use codeless_tools::attachment::{
    find_attachment_refs, reconcile_attachment_refs, AttachmentExtractError,
    AttachmentReconcileError,
};
use codeless_types::{AssistantAttachmentCard, AssistantThreadId};
use serde_json::Value;

use super::InProcessRpc;

/// Build an [`AssistantAttachmentCard`] from one tool call's output.
/// Returns `Ok(None)` when the tool's schema declares no attachments
/// or when the call produced none -- both are valid outcomes and the
/// caller persists the tool's prose result without a card.
///
/// Errors map to `RpcError` so the caller can attach the failure to
/// the in-flight Assistant turn rather than panicking the agent loop:
/// a malformed `AttachmentRef`, a cross-thread id, or a dangling id
/// are all callable bugs in plugin code, not server faults.
pub async fn build_attachment_card(
    rpc: &InProcessRpc,
    thread_id: AssistantThreadId,
    output_schema: &Value,
    output_value: &Value,
) -> RpcResult<Option<AssistantAttachmentCard>> {
    let refs = find_attachment_refs(output_schema, output_value).map_err(map_extract_err)?;
    if refs.is_empty() {
        return Ok(None);
    }

    // Lookups run inside an async block; the reconciler is sync over a
    // closure, so we materialise the rows up front and let the closure
    // consult the in-memory map. The N is bounded by the number of
    // refs the tool returned in one call -- single-digit in practice
    // -- so the per-row query cost is acceptable and keeps the
    // reconciler signature DB-free.
    let mut rows = std::collections::HashMap::new();
    for r in &refs {
        if let Some(row) = rpc
            .store
            .get_assistant_attachment(r.attachment_id)
            .await
            .map_err(super::db_err)?
        {
            rows.insert(r.attachment_id, row);
        }
    }

    let card = reconcile_attachment_refs(&refs, thread_id, |id| rows.get(&id).cloned())
        .map_err(map_reconcile_err)?;
    Ok(Some(card))
}

fn map_extract_err(e: AttachmentExtractError) -> RpcError {
    // Every extract failure is a plugin-side schema/value mismatch
    // (tool returned the wrong shape for the slot it declared). Surface
    // as `InvalidArgument` -- the call did happen, but the response
    // cannot be rendered.
    RpcError::InvalidArgument(format!(
        "tool output did not match declared attachment schema: {e}"
    ))
}

fn map_reconcile_err(e: AttachmentReconcileError) -> RpcError {
    match e {
        AttachmentReconcileError::Unknown(id) => {
            RpcError::NotFound(format!("attachment {id} (referenced by tool output)"))
        }
        AttachmentReconcileError::CrossThread {
            attachment_id,
            expected,
            actual,
        } => RpcError::InvalidArgument(format!(
            "tool returned attachment {attachment_id} from thread {actual}; \
             current thread is {expected}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::{CreateAssistantThreadArgs, RpcServer, UploadAssistantAttachmentArgs};
    use codeless_tools::attachment::{attachment_array_schema, attachment_ref_schema};
    use serde_json::json;
    use tempfile::TempDir;

    async fn rpc_with_thread() -> (InProcessRpc, AssistantThreadId, TempDir) {
        let dir = TempDir::new().unwrap();
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_assistant_data_dir(dir.path().to_path_buf());
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs {
                title: None,
                persona_id: "builtin:general".into(),
            })
            .await
            .unwrap();
        (rpc, thread.id, dir)
    }

    #[tokio::test]
    async fn empty_schema_yields_no_card() {
        let (rpc, thread_id, _dir) = rpc_with_thread().await;
        let schema = json!({});
        let value = json!({ "msg": "hello" });
        let card = build_attachment_card(&rpc, thread_id, &schema, &value)
            .await
            .unwrap();
        assert!(card.is_none());
    }

    #[tokio::test]
    async fn root_attachment_is_reconciled_against_store() {
        let (rpc, thread_id, _dir) = rpc_with_thread().await;
        // STANDARD base64 of "hi".
        let upload = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id,
                filename: "quote.pdf".into(),
                mime_type: Some("application/pdf".into()),
                content_b64: "aGk=".into(),
            })
            .await
            .unwrap();
        let stored = upload.attachment;

        let schema = attachment_ref_schema();
        // Tool lies about both mime + filename. Stored row wins.
        let value = json!({
            "attachment_id": stored.id.to_string(),
            "mime": "text/plain",
            "filename": "lies.txt",
        });
        let card = build_attachment_card(&rpc, thread_id, &schema, &value)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.kind, AssistantAttachmentCard::META_KIND);
        assert_eq!(card.items.len(), 1);
        assert_eq!(card.items[0].filename, "quote.pdf");
        assert_eq!(card.items[0].mime.as_deref(), Some("application/pdf"));
        assert_eq!(card.items[0].size_bytes, stored.size_bytes);
    }

    #[tokio::test]
    async fn array_schema_collects_every_item() {
        let (rpc, thread_id, _dir) = rpc_with_thread().await;
        let a = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id,
                filename: "one.txt".into(),
                mime_type: None,
                content_b64: "YQ==".into(),
            })
            .await
            .unwrap()
            .attachment;
        let b = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id,
                filename: "two.txt".into(),
                mime_type: None,
                content_b64: "Yg==".into(),
            })
            .await
            .unwrap()
            .attachment;

        let schema = attachment_array_schema();
        let value = json!([
            { "attachment_id": a.id.to_string() },
            { "attachment_id": b.id.to_string() },
        ]);
        let card = build_attachment_card(&rpc, thread_id, &schema, &value)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.items.len(), 2);
        assert_eq!(card.items[0].filename, "one.txt");
        assert_eq!(card.items[1].filename, "two.txt");
    }

    #[tokio::test]
    async fn unknown_id_is_not_found() {
        let (rpc, thread_id, _dir) = rpc_with_thread().await;
        let schema = attachment_ref_schema();
        let value = json!({
            "attachment_id": codeless_types::AssistantAttachmentId::new().to_string(),
        });
        let err = build_attachment_card(&rpc, thread_id, &schema, &value)
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn cross_thread_id_is_invalid_argument() {
        let (rpc, thread_a, _dir) = rpc_with_thread().await;
        let thread_b = rpc
            .create_assistant_thread(CreateAssistantThreadArgs {
                title: None,
                persona_id: "builtin:general".into(),
            })
            .await
            .unwrap()
            .id;
        let foreign = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: thread_b,
                filename: "other.txt".into(),
                mime_type: None,
                content_b64: "eA==".into(),
            })
            .await
            .unwrap()
            .attachment;

        let schema = attachment_ref_schema();
        let value = json!({ "attachment_id": foreign.id.to_string() });
        let err = build_attachment_card(&rpc, thread_a, &schema, &value)
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }
}
