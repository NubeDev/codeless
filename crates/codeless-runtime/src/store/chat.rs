use codeless_types::{
    ChatBinding, ChatMessage, ChatRole, ChatTransport, JobId, MessageId, UnixMillis,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::codec::parse_id;
use super::SqliteStore;

/// Wire-name mapping for the `transport` column. Lowercase ASCII per
/// `JOB-CHAT.md` "Wire-name convention for `ChatTransport`" — the
/// column and JSON share the exact same labels so the round-trip is
/// the same five strings everywhere.
pub(super) fn transport_label(t: ChatTransport) -> &'static str {
    match t {
        ChatTransport::Web => "web",
        ChatTransport::Cli => "cli",
        ChatTransport::Telegram => "telegram",
        ChatTransport::Slack => "slack",
        ChatTransport::Supervisor => "supervisor",
    }
}

fn parse_transport(s: &str) -> sqlx::Result<ChatTransport> {
    Ok(match s {
        "web" => ChatTransport::Web,
        "cli" => ChatTransport::Cli,
        "telegram" => ChatTransport::Telegram,
        "slack" => ChatTransport::Slack,
        "supervisor" => ChatTransport::Supervisor,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown chat transport: {other}").into(),
            ))
        }
    })
}

pub(super) fn role_label(r: ChatRole) -> &'static str {
    match r {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
        ChatRole::System => "system",
    }
}

fn parse_role(s: &str) -> sqlx::Result<ChatRole> {
    Ok(match s {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        "tool" => ChatRole::Tool,
        "system" => ChatRole::System,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown chat role: {other}").into(),
            ))
        }
    })
}

fn chat_message_from_row(row: SqliteRow) -> sqlx::Result<ChatMessage> {
    let id: String = row.try_get("id")?;
    let job_id: String = row.try_get("job_id")?;
    let transport: String = row.try_get("transport")?;
    let role: String = row.try_get("role")?;
    Ok(ChatMessage {
        id: parse_id(&id)?,
        job_id: parse_id(&job_id)?,
        run_id: row.try_get("run_id")?,
        transport: parse_transport(&transport)?,
        external_id: row.try_get("external_id")?,
        thread_key: row.try_get("thread_key")?,
        author: row.try_get("author")?,
        role: parse_role(&role)?,
        body: row.try_get("body")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: UnixMillis(row.try_get("created_at")?),
    })
}

fn chat_binding_from_row(row: SqliteRow) -> sqlx::Result<ChatBinding> {
    let transport: String = row.try_get("transport")?;
    let job_id: String = row.try_get("job_id")?;
    Ok(ChatBinding {
        transport: parse_transport(&transport)?,
        channel_id: row.try_get("channel_id")?,
        thread_id: row.try_get("thread_id")?,
        job_id: parse_id(&job_id)?,
        bound_at: UnixMillis(row.try_get("bound_at")?),
        bound_by: row.try_get("bound_by")?,
    })
}

/// Status of a `(transport, external_id)` duplicate-ingest check.
/// `Inserted` is the happy path; `DuplicateExternalId` is the
/// partial-unique-index conflict path the Telegram/Slack adapter
/// recognises as "already ingested" rather than a hard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertChatMessage {
    Inserted,
    DuplicateExternalId,
}

impl SqliteStore {
    pub async fn insert_chat_message(&self, msg: &ChatMessage) -> sqlx::Result<InsertChatMessage> {
        let result = sqlx::query(
            "INSERT INTO chat_messages \
             (id, job_id, run_id, transport, external_id, thread_key, \
              author, role, body, metadata_json, created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(msg.id.to_string())
        .bind(msg.job_id.to_string())
        .bind(&msg.run_id)
        .bind(transport_label(msg.transport))
        .bind(&msg.external_id)
        .bind(&msg.thread_key)
        .bind(&msg.author)
        .bind(role_label(msg.role))
        .bind(&msg.body)
        .bind(&msg.metadata_json)
        .bind(msg.created_at.0)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => Ok(InsertChatMessage::Inserted),
            // The partial unique index on (transport, external_id)
            // surfaces as a SQLite UNIQUE violation when an adapter
            // redelivers an inbound it already wrote. Map that to a
            // typed `DuplicateExternalId` so the caller can return
            // `Conflict` without sniffing error strings.
            Err(sqlx::Error::Database(db)) if is_unique_violation(db.as_ref()) => {
                Ok(InsertChatMessage::DuplicateExternalId)
            }
            Err(e) => Err(e),
        }
    }

    /// Page newest-first by `created_at` (id as tiebreaker so two rows
    /// minted in the same millisecond stay deterministically ordered).
    /// The returned `Vec` is reversed to oldest-first before return so
    /// the UI's renderer paints top-to-bottom without sorting; the
    /// pagination cursor in `before` still walks backward in time.
    pub async fn list_chat_messages(
        &self,
        job_id: JobId,
        before: Option<MessageId>,
        limit: u32,
    ) -> sqlx::Result<Vec<ChatMessage>> {
        let limit = limit as i64;
        let rows = match before {
            None => {
                sqlx::query(
                    "SELECT * FROM chat_messages \
                     WHERE job_id = ? \
                     ORDER BY created_at DESC, id DESC LIMIT ?",
                )
                .bind(job_id.to_string())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            Some(cursor) => {
                let cursor_row = sqlx::query("SELECT created_at FROM chat_messages WHERE id = ?")
                    .bind(cursor.to_string())
                    .fetch_optional(&self.pool)
                    .await?;
                // Unknown cursor → return an empty page rather than
                // error; the cursor came from the caller, and a
                // missing row simply means "nothing older than what
                // you've already seen" given how the partial-id walk
                // is used by transport cold-load callers.
                let Some(cursor_row) = cursor_row else {
                    return Ok(Vec::new());
                };
                let cursor_ts: i64 = cursor_row.try_get("created_at")?;
                sqlx::query(
                    "SELECT * FROM chat_messages \
                     WHERE job_id = ? \
                       AND (created_at < ? \
                            OR (created_at = ? AND id < ?)) \
                     ORDER BY created_at DESC, id DESC LIMIT ?",
                )
                .bind(job_id.to_string())
                .bind(cursor_ts)
                .bind(cursor_ts)
                .bind(cursor.to_string())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut messages: Vec<ChatMessage> = rows
            .into_iter()
            .map(chat_message_from_row)
            .collect::<sqlx::Result<_>>()?;
        messages.reverse();
        Ok(messages)
    }

    /// Idempotent upsert keyed by the row's PK
    /// `(transport, channel_id, thread_id)`. On conflict the existing
    /// row's `job_id`, `bound_at`, and `bound_by` are overwritten so a
    /// channel can be re-pointed at a different Job (the user typed
    /// `/codeless bind <other-job>` after the first bind).
    pub async fn upsert_chat_binding(&self, binding: &ChatBinding) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO chat_bindings \
             (transport, channel_id, thread_id, job_id, bound_at, bound_by) \
             VALUES (?,?,?,?,?,?) \
             ON CONFLICT(transport, channel_id, thread_id) DO UPDATE SET \
               job_id = excluded.job_id, \
               bound_at = excluded.bound_at, \
               bound_by = excluded.bound_by",
        )
        .bind(transport_label(binding.transport))
        .bind(&binding.channel_id)
        .bind(&binding.thread_id)
        .bind(binding.job_id.to_string())
        .bind(binding.bound_at.0)
        .bind(&binding.bound_by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read one binding back. Only used by transport adapters that
    /// receive an inbound message and need to resolve it to a Job —
    /// the runtime keeps no in-memory cache so a fresh adapter boot
    /// sees the current state.
    pub async fn get_chat_binding(
        &self,
        transport: ChatTransport,
        channel_id: &str,
        thread_id: &str,
    ) -> sqlx::Result<Option<ChatBinding>> {
        let row = sqlx::query(
            "SELECT * FROM chat_bindings \
             WHERE transport = ? AND channel_id = ? AND thread_id = ?",
        )
        .bind(transport_label(transport))
        .bind(channel_id)
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(chat_binding_from_row).transpose()
    }
}

fn is_unique_violation(err: &dyn sqlx::error::DatabaseError) -> bool {
    // sqlite primary error code 19 = SQLITE_CONSTRAINT; the extended
    // codes that matter here are 2067 (UNIQUE) and 1555
    // (PRIMARY KEY). Match on the extended code via the textual
    // SQLSTATE-ish path sqlx exposes; the kind() check is the
    // portable safety net for older sqlx versions where the code
    // string is not populated.
    if matches!(err.kind(), sqlx::error::ErrorKind::UniqueViolation) {
        return true;
    }
    err.code()
        .map(|c| c == "2067" || c == "1555" || c == "19")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::MIGRATOR;
    use codeless_types::{CostCents, GitAuth, Job, JobStatus, Repo, RepoId, WorkspaceMode};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_store() -> (SqliteStore, JobId) {
        // Single-connection in-memory pool — sqlx's pool keeps the
        // dedicated connection alive across queries so the schema and
        // rows survive between calls.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool);
        let now = UnixMillis(0);
        let repo = Repo {
            id: RepoId::new(),
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: GitAuth::Ssh {
                key_path: "/tmp/k".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: now,
            updated_at: now,
        };
        store.insert_repo(&repo).await.unwrap();
        let job = Job {
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "b".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: now,
        };
        store.insert_job(&job).await.unwrap();
        (store, job.id)
    }

    fn web_message(job_id: JobId, ts: i64, body: &str) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(),
            job_id,
            run_id: None,
            transport: ChatTransport::Web,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: body.into(),
            metadata_json: None,
            created_at: UnixMillis(ts),
        }
    }

    #[tokio::test]
    async fn post_then_list_roundtrip_preserves_order_and_fields() {
        let (store, job_id) = fresh_store().await;
        let a = web_message(job_id, 100, "first");
        let b = web_message(job_id, 200, "second");
        let c = web_message(job_id, 300, "third");
        for m in [&a, &b, &c] {
            assert_eq!(
                store.insert_chat_message(m).await.unwrap(),
                InsertChatMessage::Inserted
            );
        }
        let page = store.list_chat_messages(job_id, None, 10).await.unwrap();
        // Oldest-first within the page (UI-friendly), per
        // `list_job_messages` contract.
        let bodies: Vec<&str> = page.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second", "third"]);
        // Field round-trip on the first row — transport, role,
        // optional columns, created_at all decode back.
        let first = &page[0];
        assert_eq!(first.id, a.id);
        assert_eq!(first.transport, ChatTransport::Web);
        assert_eq!(first.role, ChatRole::User);
        assert_eq!(first.external_id, None);
        assert_eq!(first.created_at, UnixMillis(100));
        assert_eq!(first.author, "alice");
    }

    #[tokio::test]
    async fn list_paginates_backwards_with_before_cursor() {
        let (store, job_id) = fresh_store().await;
        let msgs: Vec<ChatMessage> = (0..5)
            .map(|i| web_message(job_id, 100 + i as i64, &format!("m{i}")))
            .collect();
        for m in &msgs {
            store.insert_chat_message(m).await.unwrap();
        }
        let page1 = store.list_chat_messages(job_id, None, 2).await.unwrap();
        let bodies1: Vec<&str> = page1.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies1, vec!["m3", "m4"]);
        // Walk back: pass the oldest id of the page as `before`.
        let oldest = page1.first().unwrap().id;
        let page2 = store
            .list_chat_messages(job_id, Some(oldest), 2)
            .await
            .unwrap();
        let bodies2: Vec<&str> = page2.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies2, vec!["m1", "m2"]);
    }

    #[tokio::test]
    async fn duplicate_external_id_on_telegram_returns_conflict_status() {
        let (store, job_id) = fresh_store().await;
        let mut m = web_message(job_id, 100, "hi");
        m.transport = ChatTransport::Telegram;
        m.external_id = Some("tg:42".into());
        assert_eq!(
            store.insert_chat_message(&m).await.unwrap(),
            InsertChatMessage::Inserted
        );
        // Redelivery: same (transport, external_id), different id.
        let dup = ChatMessage {
            id: MessageId::new(),
            ..m.clone()
        };
        assert_eq!(
            store.insert_chat_message(&dup).await.unwrap(),
            InsertChatMessage::DuplicateExternalId
        );
        // A NULL-external_id row on the same transport must NOT
        // collide — the partial unique index is conditional on
        // external_id IS NOT NULL.
        let m_null = ChatMessage {
            id: MessageId::new(),
            external_id: None,
            ..m
        };
        assert_eq!(
            store.insert_chat_message(&m_null).await.unwrap(),
            InsertChatMessage::Inserted
        );
    }

    #[tokio::test]
    async fn bind_chat_thread_is_idempotent_on_pk_and_overwrites_metadata() {
        let (store, job_id) = fresh_store().await;
        let b1 = ChatBinding {
            transport: ChatTransport::Telegram,
            channel_id: "C1".into(),
            thread_id: "".into(),
            job_id,
            bound_at: UnixMillis(1),
            bound_by: "@alice".into(),
        };
        store.upsert_chat_binding(&b1).await.unwrap();
        let b2 = ChatBinding {
            bound_at: UnixMillis(2),
            bound_by: "@bob".into(),
            ..b1.clone()
        };
        store.upsert_chat_binding(&b2).await.unwrap();
        let got = store
            .get_chat_binding(ChatTransport::Telegram, "C1", "")
            .await
            .unwrap()
            .expect("binding present");
        assert_eq!(got.bound_at, UnixMillis(2));
        assert_eq!(got.bound_by, "@bob");
        assert_eq!(got.job_id, job_id);
    }
}
