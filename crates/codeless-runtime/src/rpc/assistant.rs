use codeless_rpc::{
    AppendAssistantMessageArgs, AppendAssistantMessageResult, CreateAssistantThreadArgs,
    DeleteAssistantThreadArgs, ListAssistantMessagesArgs, ListAssistantMessagesResult,
    ListAssistantThreadsArgs, ListAssistantThreadsResult, RpcError, RpcResult,
    UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult,
};
use codeless_types::{
    AssistantAttachment, AssistantAttachmentId, AssistantMessage, AssistantMessageId,
    AssistantMessageRole, AssistantThread, AssistantThreadId,
};

use super::InProcessRpc;
use crate::time::now_ms;

const DEFAULT_THREAD_TITLE: &str = "New thread";

pub(super) async fn list_assistant_threads(
    rpc: &InProcessRpc,
    _args: ListAssistantThreadsArgs,
) -> RpcResult<ListAssistantThreadsResult> {
    let threads = rpc
        .store
        .list_assistant_threads()
        .await
        .map_err(super::db_err)?;
    Ok(ListAssistantThreadsResult { threads })
}

pub(super) async fn create_assistant_thread(
    rpc: &InProcessRpc,
    args: CreateAssistantThreadArgs,
) -> RpcResult<AssistantThread> {
    let now = now_ms();
    let title = args
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_THREAD_TITLE)
        .to_owned();
    let thread = AssistantThread {
        id: AssistantThreadId::new(),
        title,
        created_at: now,
        updated_at: now,
    };
    rpc.store
        .insert_assistant_thread(&thread)
        .await
        .map_err(super::db_err)?;
    Ok(thread)
}

pub(super) async fn delete_assistant_thread(
    rpc: &InProcessRpc,
    args: DeleteAssistantThreadArgs,
) -> RpcResult<()> {
    let removed = rpc
        .store
        .delete_assistant_thread(args.thread_id)
        .await
        .map_err(super::db_err)?;
    if !removed {
        return Err(RpcError::NotFound(format!(
            "assistant thread {}",
            args.thread_id
        )));
    }

    // Best-effort directory cleanup. The row is gone either way; a
    // residual blob on disk is harmless except for the bytes it
    // occupies. We log instead of failing so a transient FS error
    // (permissions, removable mount unmounted) does not surface as
    // an RPC failure for a row that has already been deleted.
    if let Some(root) = rpc.assistant_data_dir.as_deref() {
        let dir = root.join("threads").join(args.thread_id.to_string());
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(
                    error = %e,
                    path = %dir.display(),
                    "failed to remove assistant thread attachments dir",
                );
            }
        }
    }

    Ok(())
}

pub(super) async fn upload_assistant_attachment(
    rpc: &InProcessRpc,
    args: UploadAssistantAttachmentArgs,
) -> RpcResult<UploadAssistantAttachmentResult> {
    use base64::Engine as _;

    let root = rpc.assistant_data_dir.as_deref().ok_or_else(|| {
        RpcError::Internal(
            "assistant data dir is not configured on this runtime; \
             upload_assistant_attachment requires `with_assistant_data_dir`"
                .to_owned(),
        )
    })?;

    let thread = rpc
        .store
        .get_assistant_thread(args.thread_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("assistant thread {}", args.thread_id)))?;

    // Reuse the job-file sanitiser so directory components, dotfiles,
    // and traversal segments are rejected identically across the
    // surfaces that take user-supplied filenames.
    let safe = crate::job_dir::sanitise_filename(&args.filename)
        .map_err(|e| RpcError::InvalidArgument(format!("filename: {e:?}")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(args.content_b64.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(args.content_b64.as_bytes())
        })
        .map_err(|e| RpcError::InvalidArgument(format!("content_b64: {e}")))?;

    let attachment_id = AssistantAttachmentId::new();
    let stored_filename = format!("{}-{}", attachment_id, safe);
    let dir = root
        .join("threads")
        .join(thread.id.to_string())
        .join("attachments");
    std::fs::create_dir_all(&dir)
        .map_err(|e| RpcError::Internal(format!("create {}: {e}", dir.display())))?;
    let abs = dir.join(&stored_filename);
    std::fs::write(&abs, &bytes)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", abs.display())))?;

    let now = now_ms();
    let attachment = AssistantAttachment {
        id: attachment_id,
        thread_id: thread.id,
        original_name: safe,
        stored_filename,
        mime_type: args.mime_type,
        size_bytes: bytes.len() as i64,
        created_at: now,
    };
    rpc.store
        .insert_assistant_attachment(&attachment)
        .await
        .map_err(super::db_err)?;
    // Touch the thread so an attachment-only interaction still bumps
    // the rail order. Ignoring the boolean: a missing row is impossible
    // here because we read it above under the same store handle.
    let _ = rpc
        .store
        .touch_assistant_thread(thread.id, now)
        .await
        .map_err(super::db_err)?;

    Ok(UploadAssistantAttachmentResult { attachment })
}

pub(super) async fn list_assistant_messages(
    rpc: &InProcessRpc,
    args: ListAssistantMessagesArgs,
) -> RpcResult<ListAssistantMessagesResult> {
    let messages = rpc
        .store
        .list_assistant_messages(args.thread_id)
        .await
        .map_err(super::db_err)?;
    Ok(ListAssistantMessagesResult { messages })
}

/// Body of the no-op stage-6 responder. Wrapping the text in a const
/// (rather than inlining) makes it trivial for tests to assert on the
/// exact reply and for later stages to grep-and-replace the responder
/// with the planner output.
const NOOP_ASSISTANT_REPLY: &str = "Assistant responder is not wired yet — message recorded. \
     The real planner lands in a later stage.";

pub(super) async fn append_assistant_message(
    rpc: &InProcessRpc,
    args: AppendAssistantMessageArgs,
) -> RpcResult<AppendAssistantMessageResult> {
    let trimmed = args.content.trim();
    if trimmed.is_empty() {
        return Err(RpcError::InvalidArgument("content is empty".to_owned()));
    }

    // Existence check up front so an unknown thread fails fast without
    // a half-written user row. The FK on `assistant_messages.thread_id`
    // would catch this too, but a 404 here is the contract the UI
    // wants — `Internal("db: …")` would be wrong for "the thread the
    // user is staring at was just deleted in another window".
    rpc.store
        .get_assistant_thread(args.thread_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("assistant thread {}", args.thread_id)))?;

    let user_now = now_ms();
    let user_message = AssistantMessage {
        id: AssistantMessageId::new(),
        thread_id: args.thread_id,
        role: AssistantMessageRole::User,
        content: args.content.clone(),
        meta_json: None,
        created_at: user_now,
    };
    rpc.store
        .insert_assistant_message(&user_message)
        .await
        .map_err(super::db_err)?;

    // Distinct timestamp for the assistant turn so the ASC ordering
    // is deterministic when the row IDs tie. `now_ms()` resolution is
    // coarse on some hosts; bumping by one millisecond is enough and
    // does not skew real-world timing telemetry.
    let assistant_now = codeless_types::UnixMillis(user_now.0.saturating_add(1));
    let assistant_message = AssistantMessage {
        id: AssistantMessageId::new(),
        thread_id: args.thread_id,
        role: AssistantMessageRole::Assistant,
        content: NOOP_ASSISTANT_REPLY.to_owned(),
        meta_json: None,
        created_at: assistant_now,
    };
    rpc.store
        .insert_assistant_message(&assistant_message)
        .await
        .map_err(super::db_err)?;

    let _ = rpc
        .store
        .touch_assistant_thread(args.thread_id, assistant_now)
        .await
        .map_err(super::db_err)?;

    Ok(AppendAssistantMessageResult {
        user_message,
        assistant_message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::RpcServer;
    use tempfile::TempDir;

    async fn rpc_with_data_dir() -> (InProcessRpc, TempDir) {
        let dir = TempDir::new().unwrap();
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_assistant_data_dir(dir.path().to_path_buf());
        (rpc, dir)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_lists_and_deletes() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let a = rpc
            .create_assistant_thread(CreateAssistantThreadArgs {
                title: Some("alpha".into()),
            })
            .await
            .unwrap();
        // Force a distinct `updated_at` so the DESC order is deterministic;
        // `now_ms()` resolution is coarse enough that two back-to-back
        // creates would otherwise tie on the timestamp and break the
        // assertion on a fast host.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        assert_eq!(b.title, DEFAULT_THREAD_TITLE);

        let list = rpc
            .list_assistant_threads(ListAssistantThreadsArgs {})
            .await
            .unwrap();
        // updated_at DESC; `b` was minted after `a`.
        assert_eq!(list.threads.len(), 2);
        assert_eq!(list.threads[0].id, b.id);
        assert_eq!(list.threads[1].id, a.id);

        rpc.delete_assistant_thread(DeleteAssistantThreadArgs { thread_id: a.id })
            .await
            .unwrap();
        let after = rpc
            .list_assistant_threads(ListAssistantThreadsArgs {})
            .await
            .unwrap();
        assert_eq!(after.threads.len(), 1);
        assert_eq!(after.threads[0].id, b.id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_unknown_thread_is_not_found() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let err = rpc
            .delete_assistant_thread(DeleteAssistantThreadArgs {
                thread_id: AssistantThreadId::new(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_writes_file_and_indexes_row() {
        use base64::Engine as _;
        let (rpc, data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();

        let body = b"hello assistant world";
        let b64 = base64::engine::general_purpose::STANDARD.encode(body);
        let res = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: thread.id,
                filename: "notes.txt".into(),
                content_b64: b64,
                mime_type: Some("text/plain".into()),
            })
            .await
            .unwrap();

        assert_eq!(res.attachment.thread_id, thread.id);
        assert_eq!(res.attachment.original_name, "notes.txt");
        assert_eq!(res.attachment.size_bytes, body.len() as i64);
        assert_eq!(res.attachment.mime_type.as_deref(), Some("text/plain"));
        assert!(res.attachment.stored_filename.ends_with("-notes.txt"));

        let on_disk = data
            .path()
            .join("threads")
            .join(thread.id.to_string())
            .join("attachments")
            .join(&res.attachment.stored_filename);
        assert!(on_disk.exists(), "{} should exist", on_disk.display());
        assert_eq!(std::fs::read(&on_disk).unwrap(), body);

        // Touch fan-out: the thread row's updated_at moved forward, so
        // re-listing shows the same single thread.
        let listed = rpc.store.list_assistant_threads().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].updated_at >= thread.updated_at);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_for_unknown_thread_is_not_found() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let err = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: AssistantThreadId::new(),
                filename: "x.txt".into(),
                content_b64: String::new(),
                mime_type: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_without_data_dir_returns_internal() {
        let rpc = InProcessRpc::new().await.unwrap();
        let err = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: AssistantThreadId::new(),
                filename: "x.txt".into(),
                content_b64: String::new(),
                mime_type: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Internal(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_cleans_attachments_dir_and_cascades_rows() {
        use base64::Engine as _;
        let (rpc, data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let body = b"payload";
        let b64 = base64::engine::general_purpose::STANDARD.encode(body);
        let _ = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: thread.id,
                filename: "a.bin".into(),
                content_b64: b64,
                mime_type: None,
            })
            .await
            .unwrap();
        let dir = data.path().join("threads").join(thread.id.to_string());
        assert!(dir.exists());

        rpc.delete_assistant_thread(DeleteAssistantThreadArgs {
            thread_id: thread.id,
        })
        .await
        .unwrap();

        assert!(!dir.exists(), "attachments dir should be gone");
        let leftover = rpc
            .store
            .list_assistant_attachments(thread.id)
            .await
            .unwrap();
        assert!(leftover.is_empty(), "FK cascade should remove rows");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_rejects_bad_filename() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let err = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: thread.id,
                filename: "../etc/passwd".into(),
                content_b64: String::new(),
                mime_type: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_persists_user_and_assistant_and_lists_them() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();

        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "hi there".into(),
            })
            .await
            .unwrap();
        assert_eq!(res.user_message.role, AssistantMessageRole::User);
        assert_eq!(res.user_message.content, "hi there");
        assert_eq!(res.assistant_message.role, AssistantMessageRole::Assistant);
        assert_eq!(res.assistant_message.content, NOOP_ASSISTANT_REPLY);

        // listMessages returns both rows in created_at-ascending order.
        let listed = rpc
            .list_assistant_messages(ListAssistantMessagesArgs {
                thread_id: thread.id,
            })
            .await
            .unwrap()
            .messages;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, res.user_message.id);
        assert_eq!(listed[1].id, res.assistant_message.id);

        // Thread updated_at was touched (rail re-sort fan-out).
        let after = rpc
            .list_assistant_threads(ListAssistantThreadsArgs {})
            .await
            .unwrap();
        assert_eq!(after.threads.len(), 1);
        assert!(after.threads[0].updated_at >= thread.updated_at);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_rejects_empty_content() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let err = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "   \n\t".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_for_unknown_thread_is_not_found() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let err = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: codeless_types::AssistantThreadId::new(),
                content: "hello".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_messages_empty_thread() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let listed = rpc
            .list_assistant_messages(ListAssistantMessagesArgs {
                thread_id: thread.id,
            })
            .await
            .unwrap()
            .messages;
        assert!(listed.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_rejects_bad_base64() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let err = rpc
            .upload_assistant_attachment(UploadAssistantAttachmentArgs {
                thread_id: thread.id,
                filename: "x.bin".into(),
                content_b64: "!!! not base64 !!!".into(),
                mime_type: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }
}
