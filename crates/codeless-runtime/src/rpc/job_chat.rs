use codeless_rpc::{
    BindChatThreadArgs, ListJobMessagesArgs, ListJobMessagesResult, PostJobMessageArgs, RpcError,
    RpcResult,
};
use codeless_types::{ChatBinding, ChatMessage, MessageId};

use super::InProcessRpc;
use crate::store::InsertChatMessage;
use crate::time::now_ms;

/// Upper bound on `list_job_messages.limit`. Picked to match the
/// largest page the web `CHAT` tab and the Telegram cold-load
/// preview need without letting a runaway caller pull the full
/// `chat_messages` table in one round trip. The transport adapters
/// that need a deeper walk paginate via `before`.
const LIST_LIMIT_MAX: u32 = 500;

pub(super) async fn post_job_message(
    rpc: &InProcessRpc,
    args: PostJobMessageArgs,
) -> RpcResult<ChatMessage> {
    // Existence-check the job up-front so a typo'd id returns
    // `NotFound` instead of relying on the SQL FK to surface as
    // `Internal`. The FK still protects the table — this is the
    // typed-error path.
    if rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .is_none()
    {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }

    let now = now_ms();
    let msg = ChatMessage {
        id: MessageId::new(),
        job_id: args.job_id,
        // run_id is left NULL until JOB-WORKFLOW (B) lands and the
        // active-Run lookup is available — JOB-CHAT.md is explicit
        // that the supervisor's reading view stays per-Job
        // regardless. See OQ-CHAT-4.
        run_id: None,
        transport: args.transport,
        external_id: args.external_id,
        thread_key: args.thread_key,
        author: args.author,
        role: args.role,
        body: args.body,
        metadata_json: args.metadata_json,
        created_at: now,
    };
    match rpc
        .store
        .insert_chat_message(&msg)
        .await
        .map_err(super::db_err)?
    {
        InsertChatMessage::Inserted => Ok(msg),
        // Redelivery of an already-ingested Telegram / Slack message.
        // Surfaced as `Conflict` so the adapter recognises the
        // duplicate-ingest defence without sniffing error strings;
        // see JOB-CHAT.md "Idempotency" on the inbound path.
        InsertChatMessage::DuplicateExternalId => Err(RpcError::Conflict(format!(
            "chat message ({:?}, {}) already ingested",
            msg.transport,
            msg.external_id.as_deref().unwrap_or("")
        ))),
    }
}

pub(super) async fn list_job_messages(
    rpc: &InProcessRpc,
    args: ListJobMessagesArgs,
) -> RpcResult<ListJobMessagesResult> {
    if rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .is_none()
    {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    if args.limit == 0 {
        return Err(RpcError::InvalidArgument(
            "list_job_messages: limit must be > 0".into(),
        ));
    }
    let limit = args.limit.min(LIST_LIMIT_MAX);
    let messages = rpc
        .store
        .list_chat_messages(args.job_id, args.before, limit)
        .await
        .map_err(super::db_err)?;
    Ok(ListJobMessagesResult { messages })
}

pub(super) async fn bind_chat_thread(
    rpc: &InProcessRpc,
    args: BindChatThreadArgs,
) -> RpcResult<ChatBinding> {
    if rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .is_none()
    {
        return Err(RpcError::NotFound(format!("job {}", args.job_id)));
    }
    let binding = ChatBinding {
        transport: args.transport,
        channel_id: args.channel_id,
        // Empty-string sentinel for "no thread on this transport"
        // matches the `chat_bindings.thread_id NOT NULL` invariant
        // that the PK relies on; see JOB-CHAT.md "Data model".
        thread_id: args.thread_id.unwrap_or_default(),
        job_id: args.job_id,
        bound_at: now_ms(),
        bound_by: args.bound_by,
    };
    rpc.store
        .upsert_chat_binding(&binding)
        .await
        .map_err(super::db_err)?;
    Ok(binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use codeless_rpc::{AddRepoArgs, RpcError, RpcServer, SubmitJobArgs};
    use codeless_types::{ChatRole, ChatTransport, GitAuth};

    async fn fresh_rpc_with_job() -> (InProcessRpc, codeless_types::JobId) {
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = rpc
            .add_repo(AddRepoArgs {
                name: "r".into(),
                clone_url: "u".into(),
                default_branch: "main".into(),
                local_path: "/tmp".into(),
                git_auth: GitAuth::Ssh {
                    key_path: "/tmp/k".into(),
                },
                concurrency_cap: None,
                default_runner: None,
            })
            .await
            .unwrap();
        let job = rpc
            .submit_job(SubmitJobArgs {
                repo_id: repo.id,
                prompt: Some("hi".into()),
                template_yaml: None,
                runner: "mock".into(),
                branch: "b".into(),
                workspace_mode: None,
                cost_cap_cents: 0,
                wall_clock_cap_ms: 0,
                model: None,
                permission_mode: None,
                effort: None,
                system_prompt: None,
                persona_id: None,
                auto_bypass_policy: None,
                start_immediately: false,
            })
            .await
            .unwrap();
        (rpc, job.id)
    }

    fn web_args(job_id: codeless_types::JobId, body: &str) -> PostJobMessageArgs {
        PostJobMessageArgs {
            job_id,
            transport: ChatTransport::Web,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: body.into(),
            metadata_json: None,
        }
    }

    #[tokio::test]
    async fn post_then_list_roundtrips_across_the_rpc_surface() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let posted = post_job_message(&rpc, web_args(job_id, "hi"))
            .await
            .unwrap();
        assert_eq!(posted.body, "hi");
        let listed = list_job_messages(
            &rpc,
            ListJobMessagesArgs {
                job_id,
                before: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
        assert_eq!(listed.messages.len(), 1);
        assert_eq!(listed.messages[0].id, posted.id);
        assert_eq!(listed.messages[0].body, "hi");
    }

    #[tokio::test]
    async fn bind_chat_thread_upsert_is_idempotent_and_overwrites_metadata() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let first = bind_chat_thread(
            &rpc,
            BindChatThreadArgs {
                transport: ChatTransport::Telegram,
                channel_id: "C1".into(),
                thread_id: None,
                job_id,
                bound_by: "@alice".into(),
            },
        )
        .await
        .unwrap();
        let second = bind_chat_thread(
            &rpc,
            BindChatThreadArgs {
                transport: ChatTransport::Telegram,
                channel_id: "C1".into(),
                thread_id: None,
                job_id,
                bound_by: "@bob".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(first.thread_id, "");
        assert_eq!(second.thread_id, "");
        assert_eq!(second.bound_by, "@bob");
        // Both calls return successfully — the second is the upsert
        // overwrite, not a Conflict. The stored row reflects the
        // latest call.
        let got = rpc
            .store
            .get_chat_binding(ChatTransport::Telegram, "C1", "")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.bound_by, "@bob");
    }

    #[tokio::test]
    async fn redelivered_external_id_returns_conflict() {
        let (rpc, job_id) = fresh_rpc_with_job().await;
        let args = PostJobMessageArgs {
            job_id,
            transport: ChatTransport::Telegram,
            external_id: Some("tg:42".into()),
            thread_key: Some("ch1".into()),
            author: "tg-user".into(),
            role: ChatRole::User,
            body: "hello".into(),
            metadata_json: None,
        };
        post_job_message(&rpc, args.clone()).await.unwrap();
        let err = post_job_message(&rpc, args).await.unwrap_err();
        assert!(matches!(err, RpcError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn unknown_job_returns_not_found_on_every_method() {
        let rpc = InProcessRpc::new().await.unwrap();
        let phantom = codeless_types::JobId::new();
        let err = post_job_message(&rpc, web_args(phantom, "x"))
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)));
        let err = list_job_messages(
            &rpc,
            ListJobMessagesArgs {
                job_id: phantom,
                before: None,
                limit: 1,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)));
        let err = bind_chat_thread(
            &rpc,
            BindChatThreadArgs {
                transport: ChatTransport::Telegram,
                channel_id: "C".into(),
                thread_id: None,
                job_id: phantom,
                bound_by: "@a".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)));
    }
}
