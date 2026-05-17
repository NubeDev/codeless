//! Tool-result attachments (`DOCS/PLUGIN-SUBSTRATE.md` item 7 / PS7).
//!
//! A plugin tool declares in its output JSON-Schema that one or more
//! result fields are *attachments* by using the marker `{"$ref":
//! "codeless://attachment"}` (or an array thereof). At call time the
//! tool's JSON return value carries an [`AttachmentRef`] (or a
//! `Vec<AttachmentRef>`) at the matching path. The Assistant agent
//! loop (PS8) walks the schema with [`find_attachment_refs`], hands
//! the collected refs to the runtime's reconciliation helper to
//! resolve advisory `mime`/`filename` hints against the stored
//! `assistant_attachments` row, and persists an
//! [`AssistantAttachmentCard`] meta payload that the UI renders as a
//! download card without any per-plugin UI code.
//!
//! This module is schema + extraction only -- no DB, no I/O. The
//! runtime owns the store lookup; keeping that boundary clean is what
//! lets `codeless-tools` stay sqlx-free (and one day plugin-host-safe).
//!
//! The schema walker is intentionally narrow: it understands the marker
//! at the root of the output value, inside a property of the root
//! object, inside an array (`"items": { "$ref": ... }`), or inside an
//! object's property declared as an array. Anything deeper than that
//! is rejected at extraction time -- if a plugin needs richer
//! attachment-bearing shapes we extend the matcher here, not in every
//! caller.

use serde_json::Value;

use codeless_types::{AssistantAttachmentCard, AssistantAttachmentCardItem, AttachmentRef};

/// JSON-Schema `$ref` URI a plugin tool uses to mark an attachment in
/// its output schema. The literal string is the contract -- changing
/// it would break every plugin manifest in the wild, so it lives in
/// one constant.
pub const ATTACHMENT_SCHEMA_REF: &str = "codeless://attachment";

/// Convenience: `{"$ref": "codeless://attachment"}`. Plugins compose
/// their output schema by dropping this object at the attachment
/// position. Returned by value because schemas are constructed once
/// at registration time; the cost is irrelevant.
pub fn attachment_ref_schema() -> Value {
    serde_json::json!({ "$ref": ATTACHMENT_SCHEMA_REF })
}

/// Convenience: `{"type": "array", "items": {"$ref":
/// "codeless://attachment"}}`. The array-of-attachments shape called
/// out in the substrate doc.
pub fn attachment_array_schema() -> Value {
    serde_json::json!({
        "type": "array",
        "items": { "$ref": ATTACHMENT_SCHEMA_REF },
    })
}

#[derive(Debug, thiserror::Error)]
pub enum AttachmentExtractError {
    #[error("attachment ref at `{path}` is not a JSON object")]
    NotAnObject { path: String },
    #[error("attachment ref at `{path}` failed to decode: {source}")]
    Decode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("schema at `{path}` declared `{ATTACHMENT_SCHEMA_REF}` but the value is not an array")]
    ExpectedArray { path: String },
    #[error("schema at `{path}` declared `{ATTACHMENT_SCHEMA_REF}` but the value is missing")]
    Missing { path: String },
}

/// Walk `schema` looking for `{"$ref": "codeless://attachment"}` markers
/// and collect every [`AttachmentRef`] the matching positions in
/// `value` carry. Returns the empty vec when the schema declares no
/// attachments.
///
/// The matcher recognises four shapes (covering everything the
/// substrate doc describes):
///
/// 1. **Whole value is one attachment.** Schema root is `{"$ref": ...}`,
///    `value` is one `AttachmentRef` object.
/// 2. **Whole value is an array of attachments.** Schema root is
///    `{"type": "array", "items": {"$ref": ...}}`, `value` is a JSON
///    array of `AttachmentRef` objects.
/// 3. **Named field is one attachment.** Schema root is an object with
///    a property whose value is `{"$ref": ...}`; `value`'s matching
///    property is one `AttachmentRef`.
/// 4. **Named field is an array of attachments.** Schema root is an
///    object with a property whose value is `{"type": "array",
///    "items": {"$ref": ...}}`; `value`'s matching property is a JSON
///    array of `AttachmentRef`s.
///
/// Anything richer (nested objects, oneOf, ...) is silently ignored at
/// this layer; the substrate doc explicitly limits the contract to the
/// shapes above so the renderer cannot be tricked into recursing into
/// arbitrary plugin output.
pub fn find_attachment_refs(
    schema: &Value,
    value: &Value,
) -> Result<Vec<AttachmentRef>, AttachmentExtractError> {
    let mut out = Vec::new();
    walk(schema, value, "$", &mut out)?;
    Ok(out)
}

fn walk(
    schema: &Value,
    value: &Value,
    path: &str,
    out: &mut Vec<AttachmentRef>,
) -> Result<(), AttachmentExtractError> {
    if is_attachment_ref_schema(schema) {
        collect_single(value, path, out)?;
        return Ok(());
    }
    if is_attachment_array_schema(schema) {
        collect_array(value, path, out)?;
        return Ok(());
    }
    // Object-with-properties: descend one level. Deeper nesting is
    // intentionally not followed -- see the doc comment above.
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        let Some(value_obj) = value.as_object() else {
            return Ok(());
        };
        for (name, prop_schema) in props {
            if let Some(v) = value_obj.get(name) {
                let child_path = format!("{path}.{name}");
                if is_attachment_ref_schema(prop_schema) {
                    collect_single(v, &child_path, out)?;
                } else if is_attachment_array_schema(prop_schema) {
                    collect_array(v, &child_path, out)?;
                }
            } else if is_attachment_ref_schema(prop_schema)
                || is_attachment_array_schema(prop_schema)
            {
                // Schema declared an attachment field but the value is
                // absent. Tolerated -- the tool legitimately may
                // produce zero attachments on a given call -- so do
                // not raise `Missing` here.
            }
        }
    }
    Ok(())
}

fn is_attachment_ref_schema(schema: &Value) -> bool {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .map(|s| s == ATTACHMENT_SCHEMA_REF)
        .unwrap_or(false)
}

fn is_attachment_array_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("array")
        && schema
            .get("items")
            .map(is_attachment_ref_schema)
            .unwrap_or(false)
}

fn collect_single(
    value: &Value,
    path: &str,
    out: &mut Vec<AttachmentRef>,
) -> Result<(), AttachmentExtractError> {
    if value.is_null() {
        return Err(AttachmentExtractError::Missing {
            path: path.to_owned(),
        });
    }
    if !value.is_object() {
        return Err(AttachmentExtractError::NotAnObject {
            path: path.to_owned(),
        });
    }
    let parsed: AttachmentRef =
        serde_json::from_value(value.clone()).map_err(|source| AttachmentExtractError::Decode {
            path: path.to_owned(),
            source,
        })?;
    out.push(parsed);
    Ok(())
}

fn collect_array(
    value: &Value,
    path: &str,
    out: &mut Vec<AttachmentRef>,
) -> Result<(), AttachmentExtractError> {
    let Some(arr) = value.as_array() else {
        return Err(AttachmentExtractError::ExpectedArray {
            path: path.to_owned(),
        });
    };
    for (i, item) in arr.iter().enumerate() {
        let child = format!("{path}[{i}]");
        collect_single(item, &child, out)?;
    }
    Ok(())
}

/// Reconcile a list of tool-supplied [`AttachmentRef`]s against the
/// authoritative `assistant_attachments` rows. `lookup` is the
/// runtime-owned accessor (a closure over `SqliteStore::
/// get_assistant_attachment`) so this helper stays free of a DB
/// dependency. Returns the reconciled card the runtime persists into
/// the message's `meta_json`.
///
/// Reconciliation rule (substrate doc item 7): the stored row is
/// authoritative for `filename`, `mime`, and `size_bytes`. The
/// tool-supplied advisory `mime`/`filename` are dropped silently if
/// they disagree with the row -- a diverging hint is not an error,
/// just unused. A `lookup` returning `None` for a referenced id *is*
/// an error: the tool returned an id the server cannot resolve, which
/// means either the row was deleted between the tool call and the
/// reconciliation (race) or the tool fabricated an id. Both are bugs;
/// surface them rather than render a phantom card.
///
/// `expected_thread` scopes every referenced attachment to the thread
/// the tool call is happening on -- a plugin tool cannot exfiltrate
/// another thread's attachments by returning an id from a different
/// thread. The check is server-side because the schema-declared marker
/// cannot enforce it (the wire shape carries no thread id).
pub fn reconcile_attachment_refs<F>(
    refs: &[AttachmentRef],
    expected_thread: codeless_types::AssistantThreadId,
    mut lookup: F,
) -> Result<AssistantAttachmentCard, AttachmentReconcileError>
where
    F: FnMut(codeless_types::AssistantAttachmentId) -> Option<codeless_types::AssistantAttachment>,
{
    let mut items = Vec::with_capacity(refs.len());
    for r in refs {
        let row =
            lookup(r.attachment_id).ok_or(AttachmentReconcileError::Unknown(r.attachment_id))?;
        if row.thread_id != expected_thread {
            return Err(AttachmentReconcileError::CrossThread {
                attachment_id: r.attachment_id,
                expected: expected_thread,
                actual: row.thread_id,
            });
        }
        items.push(AssistantAttachmentCardItem {
            attachment_id: row.id,
            filename: row.original_name,
            mime: row.mime_type,
            size_bytes: row.size_bytes,
        });
    }
    Ok(AssistantAttachmentCard::new(items))
}

#[derive(Debug, thiserror::Error)]
pub enum AttachmentReconcileError {
    #[error("attachment {0} is not present in the store")]
    Unknown(codeless_types::AssistantAttachmentId),
    #[error(
        "attachment {attachment_id} belongs to thread {actual}, not the current thread {expected}"
    )]
    CrossThread {
        attachment_id: codeless_types::AssistantAttachmentId,
        expected: codeless_types::AssistantThreadId,
        actual: codeless_types::AssistantThreadId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{AssistantAttachment, AssistantAttachmentId, AssistantThreadId};
    use serde_json::json;

    fn att_row(thread: AssistantThreadId, name: &str) -> AssistantAttachment {
        AssistantAttachment {
            id: AssistantAttachmentId::new(),
            thread_id: thread,
            original_name: name.to_owned(),
            stored_filename: format!("stored-{name}"),
            mime_type: Some("application/pdf".to_owned()),
            size_bytes: 17,
            created_at: codeless_types::UnixMillis(0),
        }
    }

    #[test]
    fn root_single_attachment() {
        let schema = attachment_ref_schema();
        let id = AssistantAttachmentId::new();
        let value = json!({ "attachment_id": id.to_string() });
        let refs = find_attachment_refs(&schema, &value).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].attachment_id, id);
        assert!(refs[0].mime.is_none());
    }

    #[test]
    fn root_array_of_attachments() {
        let schema = attachment_array_schema();
        let a = AssistantAttachmentId::new();
        let b = AssistantAttachmentId::new();
        let value = json!([
            { "attachment_id": a.to_string(), "mime": "application/pdf" },
            { "attachment_id": b.to_string(), "filename": "report.docx" },
        ]);
        let refs = find_attachment_refs(&schema, &value).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].attachment_id, a);
        assert_eq!(refs[0].mime.as_deref(), Some("application/pdf"));
        assert_eq!(refs[1].attachment_id, b);
        assert_eq!(refs[1].filename.as_deref(), Some("report.docx"));
    }

    #[test]
    fn property_single_and_array() {
        let schema = json!({
            "type": "object",
            "properties": {
                "pdf": { "$ref": ATTACHMENT_SCHEMA_REF },
                "attachments": {
                    "type": "array",
                    "items": { "$ref": ATTACHMENT_SCHEMA_REF },
                },
                "summary": { "type": "string" },
            }
        });
        let id1 = AssistantAttachmentId::new();
        let id2 = AssistantAttachmentId::new();
        let value = json!({
            "pdf": { "attachment_id": id1.to_string() },
            "attachments": [
                { "attachment_id": id2.to_string() }
            ],
            "summary": "ignored",
        });
        let refs = find_attachment_refs(&schema, &value).unwrap();
        let ids: Vec<_> = refs.iter().map(|r| r.attachment_id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn missing_property_is_tolerated() {
        let schema = json!({
            "type": "object",
            "properties": {
                "pdf": { "$ref": ATTACHMENT_SCHEMA_REF },
            }
        });
        let value = json!({});
        let refs = find_attachment_refs(&schema, &value).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn array_schema_with_scalar_value_errors() {
        let schema = attachment_array_schema();
        let value = json!("not an array");
        let err = find_attachment_refs(&schema, &value).unwrap_err();
        assert!(matches!(err, AttachmentExtractError::ExpectedArray { .. }));
    }

    #[test]
    fn single_with_scalar_value_errors() {
        let schema = attachment_ref_schema();
        let value = json!("nope");
        let err = find_attachment_refs(&schema, &value).unwrap_err();
        assert!(matches!(err, AttachmentExtractError::NotAnObject { .. }));
    }

    #[test]
    fn reconcile_drops_advisory_hints_in_favour_of_stored_row() {
        let thread = AssistantThreadId::new();
        let row = att_row(thread, "real.pdf");
        let row_id = row.id;
        let refs = vec![AttachmentRef {
            attachment_id: row_id,
            // Tool lied about both: server stored mime+name win.
            mime: Some("text/plain".to_owned()),
            filename: Some("fake.txt".to_owned()),
        }];
        let card = reconcile_attachment_refs(&refs, thread, |id| {
            if id == row_id {
                Some(row.clone())
            } else {
                None
            }
        })
        .unwrap();
        assert_eq!(card.kind, AssistantAttachmentCard::META_KIND);
        assert_eq!(card.items.len(), 1);
        assert_eq!(card.items[0].filename, "real.pdf");
        assert_eq!(card.items[0].mime.as_deref(), Some("application/pdf"));
        assert_eq!(card.items[0].size_bytes, 17);
    }

    #[test]
    fn reconcile_unknown_id_errors() {
        let thread = AssistantThreadId::new();
        let refs = vec![AttachmentRef {
            attachment_id: AssistantAttachmentId::new(),
            mime: None,
            filename: None,
        }];
        let err = reconcile_attachment_refs(&refs, thread, |_| None).unwrap_err();
        assert!(matches!(err, AttachmentReconcileError::Unknown(_)));
    }

    #[test]
    fn reconcile_rejects_cross_thread_id() {
        let thread_a = AssistantThreadId::new();
        let thread_b = AssistantThreadId::new();
        let row = att_row(thread_b, "leaked.pdf");
        let row_id = row.id;
        let refs = vec![AttachmentRef {
            attachment_id: row_id,
            mime: None,
            filename: None,
        }];
        let err = reconcile_attachment_refs(&refs, thread_a, |id| {
            if id == row_id {
                Some(row.clone())
            } else {
                None
            }
        })
        .unwrap_err();
        assert!(matches!(err, AttachmentReconcileError::CrossThread { .. }));
    }
}
