use codeless_types::{
    AssistantAttachment, AssistantMessage, AssistantMessageId, AssistantThread, AssistantThreadId,
    AssistantThreadMode, UnixMillis,
};

use super::codec::{
    assistant_attachment_from_row, assistant_message_from_row, assistant_role_label,
    assistant_thread_from_row,
};
use super::SqliteStore;

impl SqliteStore {
    pub async fn insert_assistant_thread(&self, thread: &AssistantThread) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_threads \
             (id, title, persona_id, mode, created_at, updated_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(thread.id.to_string())
        .bind(&thread.title)
        .bind(&thread.persona_id)
        .bind(thread.mode.as_wire())
        .bind(thread.created_at.0)
        .bind(thread.updated_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Flip an assistant thread's `mode` column (job
    /// `assistant-fs-tools` stage 3). Returns `true` when a row was
    /// updated; `false` lets the RPC distinguish "no such thread"
    /// from "mode unchanged" without a second `SELECT`. `updated_at`
    /// is *not* bumped — switching permission posture is not a
    /// conversational event and should not re-sort the rail.
    pub async fn set_assistant_thread_mode(
        &self,
        id: AssistantThreadId,
        mode: AssistantThreadMode,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE assistant_threads SET mode = ? WHERE id = ?")
            .bind(mode.as_wire())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_assistant_thread(
        &self,
        id: AssistantThreadId,
    ) -> sqlx::Result<Option<AssistantThread>> {
        let row = sqlx::query("SELECT * FROM assistant_threads WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(assistant_thread_from_row).transpose()
    }

    /// List every assistant thread, newest-touched first. The query
    /// uses the `assistant_threads_updated_idx` (DESC) so the rail
    /// renders in stable order without a runtime sort.
    pub async fn list_assistant_threads(&self) -> sqlx::Result<Vec<AssistantThread>> {
        let rows = sqlx::query("SELECT * FROM assistant_threads ORDER BY updated_at DESC, id DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(assistant_thread_from_row).collect()
    }

    /// Delete a thread. `assistant_messages` and `assistant_attachments`
    /// cascade automatically via the FK; callers handle the on-disk
    /// attachments directory separately because the store has no
    /// filesystem handle. Returns `true` when a row was removed.
    pub async fn delete_assistant_thread(&self, id: AssistantThreadId) -> sqlx::Result<bool> {
        let res = sqlx::query("DELETE FROM assistant_threads WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Stamp `updated_at` on a thread row without otherwise touching
    /// it. Called after a message append or attachment upload so the
    /// rail order reflects activity. No-op when the id is unknown.
    pub async fn touch_assistant_thread(
        &self,
        id: AssistantThreadId,
        when: UnixMillis,
    ) -> sqlx::Result<bool> {
        let res = sqlx::query("UPDATE assistant_threads SET updated_at = ? WHERE id = ?")
            .bind(when.0)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn insert_assistant_message(&self, message: &AssistantMessage) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, thread_id, role, content, meta_json, created_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(message.id.to_string())
        .bind(message.thread_id.to_string())
        .bind(assistant_role_label(message.role))
        .bind(&message.content)
        .bind(&message.meta_json)
        .bind(message.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch one message by id. Used by the action-card confirm/cancel
    /// path so the runtime can re-parse `meta_json` server-side rather
    /// than trust the client's claim about what's pending. Returns
    /// `None` when the row is missing (already gone, or never existed).
    pub async fn get_assistant_message(
        &self,
        id: AssistantMessageId,
    ) -> sqlx::Result<Option<AssistantMessage>> {
        let row = sqlx::query("SELECT * FROM assistant_messages WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(assistant_message_from_row).transpose()
    }

    /// Replace `meta_json` and `content` on an existing message. The
    /// action-card flow uses this to flip the status of a proposal row
    /// in place — keeping the same id and `created_at` means the rail
    /// re-renders the card with new buttons (or none) instead of
    /// growing a duplicate entry. Returns `false` if the row is gone.
    pub async fn update_assistant_message(
        &self,
        id: AssistantMessageId,
        content: &str,
        meta_json: Option<&str>,
    ) -> sqlx::Result<bool> {
        let res =
            sqlx::query("UPDATE assistant_messages SET content = ?, meta_json = ? WHERE id = ?")
                .bind(content)
                .bind(meta_json)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_assistant_messages(
        &self,
        thread_id: AssistantThreadId,
    ) -> sqlx::Result<Vec<AssistantMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_messages \
             WHERE thread_id = ? \
             ORDER BY created_at, id",
        )
        .bind(thread_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(assistant_message_from_row).collect()
    }

    pub async fn insert_assistant_attachment(&self, att: &AssistantAttachment) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO assistant_attachments \
             (id, thread_id, original_name, stored_filename, mime_type, size_bytes, created_at) \
             VALUES (?,?,?,?,?,?,?)",
        )
        .bind(att.id.to_string())
        .bind(att.thread_id.to_string())
        .bind(&att.original_name)
        .bind(&att.stored_filename)
        .bind(&att.mime_type)
        .bind(att.size_bytes)
        .bind(att.created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch one attachment row by id. Used by the PS7 reconciliation
    /// helper to resolve a tool-returned `AttachmentRef` against the
    /// stored row (filename / mime / size become the authoritative
    /// values the UI renders; tool-supplied advisory hints are
    /// dropped). Returns `None` for a missing or deleted row -- the
    /// caller surfaces that as an `AttachmentReconcileError::Unknown`.
    pub async fn get_assistant_attachment(
        &self,
        id: codeless_types::AssistantAttachmentId,
    ) -> sqlx::Result<Option<AssistantAttachment>> {
        let row = sqlx::query("SELECT * FROM assistant_attachments WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(assistant_attachment_from_row).transpose()
    }

    pub async fn list_assistant_attachments(
        &self,
        thread_id: AssistantThreadId,
    ) -> sqlx::Result<Vec<AssistantAttachment>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_attachments \
             WHERE thread_id = ? \
             ORDER BY created_at, id",
        )
        .bind(thread_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(assistant_attachment_from_row)
            .collect()
    }
}
