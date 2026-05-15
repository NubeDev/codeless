use codeless_rpc::{
    AppendAssistantMessageArgs, AppendAssistantMessageResult, CancelAssistantActionArgs,
    CancelAssistantActionResult, ConfirmAssistantActionArgs, ConfirmAssistantActionResult,
    CreateAssistantThreadArgs, DeleteAssistantThreadArgs, GetJobArgs, ListAssistantMessagesArgs,
    ListAssistantMessagesResult, ListAssistantThreadsArgs, ListAssistantThreadsResult,
    ListJobsArgs, PauseJobArgs, RerunJobArgs, ResumeJobArgs, RpcError, RpcResult, RpcServer,
    StartJobArgs, StopJobArgs, UpdateJobArgs, UploadAssistantAttachmentArgs,
    UploadAssistantAttachmentResult,
};
use codeless_types::{
    AssistantAction, AssistantActionCard, AssistantActionStatus, AssistantAttachment,
    AssistantAttachmentId, AssistantMessage, AssistantMessageId, AssistantMessageRole,
    AssistantThread, AssistantThreadId, JobId, RepoId,
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
    // Slash-command parser stands in for the planner: stage 7 lands
    // the action-card surface before the planner is wired, so a
    // user can drive view/manage tools by typing `/start <job_id>`
    // (etc.) and confirming the proposal. Anything that does not
    // match falls through to the no-op acknowledgement so the
    // transcript stays useful while the planner is still missing.
    let (content, meta_json) = match parse_action(trimmed) {
        Some((action, summary)) => {
            let card = AssistantActionCard::new(action);
            let meta = serde_json::to_string(&card)
                .map_err(|e| RpcError::Internal(format!("serialise action card: {e}")))?;
            (summary, Some(meta))
        }
        None => (NOOP_ASSISTANT_REPLY.to_owned(), None),
    };
    let assistant_message = AssistantMessage {
        id: AssistantMessageId::new(),
        thread_id: args.thread_id,
        role: AssistantMessageRole::Assistant,
        content,
        meta_json,
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

/// Parse a user turn into an action proposal. Stage-7 stand-in for the
/// real planner: a tiny slash-command DSL covering the view/manage
/// surface listed in `DOCS/ASSISTANT-SCOPE.md`. Returns the typed
/// action plus the human summary that becomes the card's `content`
/// fallback. `None` means "not an action" — caller falls back to the
/// no-op responder so plain chat still echoes acknowledgement.
///
/// The grammar deliberately mirrors the RPC method names so the user
/// can guess them without docs:
///
/// ```text
/// /list-jobs [<repo_id>]
/// /get <job_id>
/// /start <job_id>
/// /stop <job_id>
/// /pause <job_id>
/// /resume <job_id>
/// /restart <job_id>
/// /update <job_id> key=value [key=value …]
/// ```
///
/// `key` is one of: `runner`, `model`, `permission_mode`, `effort`,
/// `cost_cap_cents`, `wall_clock_cap_ms`, `branch`. Unknown keys
/// fail the parse so the user sees the no-op reply rather than a
/// silently-dropped patch.
fn parse_action(input: &str) -> Option<(AssistantAction, String)> {
    let line = input.trim();
    let rest = line.strip_prefix('/')?;
    let mut parts = rest.split_whitespace();
    let cmd = parts.next()?;
    match cmd {
        "list-jobs" | "list" | "jobs" => {
            let repo_id = parts.next().and_then(|s| s.parse::<RepoId>().ok());
            let summary = match &repo_id {
                Some(r) => format!("List jobs for repo `{r}`"),
                None => "List jobs across every repo".to_owned(),
            };
            Some((AssistantAction::ListJobs { repo_id }, summary))
        }
        "get" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::GetJob { job_id },
                format!("Get job `{job_id}`"),
            ))
        }
        "start" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::StartJob { job_id },
                format!("Start job `{job_id}` (Draft → Queued)"),
            ))
        }
        "stop" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::StopJob { job_id },
                format!("Stop job `{job_id}`"),
            ))
        }
        "pause" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::PauseJob { job_id },
                format!("Pause job `{job_id}`"),
            ))
        }
        "resume" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::ResumeJob { job_id },
                format!("Resume job `{job_id}`"),
            ))
        }
        "restart" | "rerun" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            Some((
                AssistantAction::RestartJob { job_id },
                format!("Restart job `{job_id}` (fresh attempt, new branch)"),
            ))
        }
        "update" => {
            let job_id = parts.next()?.parse::<JobId>().ok()?;
            let mut runner = None;
            let mut model = None;
            let mut permission_mode = None;
            let mut effort = None;
            let mut cost_cap_cents = None;
            let mut wall_clock_cap_ms = None;
            let mut branch = None;
            let mut summary_pairs = Vec::new();
            for tok in parts {
                let (k, v) = tok.split_once('=')?;
                match k {
                    "runner" => runner = Some(v.to_owned()),
                    "model" => model = Some(v.to_owned()),
                    "permission_mode" => permission_mode = Some(v.to_owned()),
                    "effort" => effort = Some(v.to_owned()),
                    "cost_cap_cents" => cost_cap_cents = Some(v.parse().ok()?),
                    "wall_clock_cap_ms" => wall_clock_cap_ms = Some(v.parse().ok()?),
                    "branch" => branch = Some(v.to_owned()),
                    _ => return None,
                }
                summary_pairs.push(format!("`{k}` → `{v}`"));
            }
            if summary_pairs.is_empty() {
                return None;
            }
            let summary = format!(
                "Update job `{job_id}`: {fields}",
                fields = summary_pairs.join(", "),
            );
            Some((
                AssistantAction::UpdateJob {
                    job_id,
                    runner,
                    model,
                    permission_mode,
                    effort,
                    cost_cap_cents,
                    wall_clock_cap_ms,
                    branch,
                },
                summary,
            ))
        }
        _ => None,
    }
}

/// Internal helper: load the proposal row and re-deserialise the card
/// from `meta_json`, with the typed errors the confirm/cancel surface
/// needs. Pulling this out of both call sites keeps the validation
/// rules ("must exist on this thread, must be a pending action card")
/// in one place — drift between confirm and cancel would let a
/// race-y double-click slip through.
async fn load_pending_card(
    rpc: &InProcessRpc,
    thread_id: AssistantThreadId,
    message_id: AssistantMessageId,
) -> RpcResult<(AssistantMessage, AssistantActionCard)> {
    let row = rpc
        .store
        .get_assistant_message(message_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("assistant message {message_id}")))?;
    if row.thread_id != thread_id {
        // Distinct thread id is "not on this thread" — semantically the
        // same as missing from the caller's point of view.
        return Err(RpcError::NotFound(format!(
            "assistant message {message_id} on thread {thread_id}"
        )));
    }
    let raw = row.meta_json.as_deref().ok_or_else(|| {
        RpcError::InvalidArgument(format!("message {message_id} is not an action card"))
    })?;
    let card: AssistantActionCard = serde_json::from_str(raw)
        .map_err(|e| RpcError::InvalidArgument(format!("message {message_id} meta_json: {e}")))?;
    if card.kind != AssistantActionCard::META_KIND {
        return Err(RpcError::InvalidArgument(format!(
            "message {message_id} is not an action card"
        )));
    }
    if !matches!(card.status, AssistantActionStatus::Pending) {
        return Err(RpcError::InvalidArgument(format!(
            "action card {message_id} is not pending (status: {:?})",
            card.status
        )));
    }
    Ok((row, card))
}

/// Re-serialise an updated card back onto its message row and return
/// the refreshed `AssistantMessage`. Keeping the write-back path in
/// one helper means the confirm-success, confirm-failure, and cancel
/// branches all flip status the same way — there is no path that
/// leaves a card half-mutated.
async fn write_card_status(
    rpc: &InProcessRpc,
    message: &AssistantMessage,
    mut card: AssistantActionCard,
    status: AssistantActionStatus,
) -> RpcResult<AssistantMessage> {
    card.status = status;
    let meta = serde_json::to_string(&card)
        .map_err(|e| RpcError::Internal(format!("serialise action card: {e}")))?;
    let ok = rpc
        .store
        .update_assistant_message(message.id, &message.content, Some(&meta))
        .await
        .map_err(super::db_err)?;
    if !ok {
        return Err(RpcError::NotFound(format!(
            "assistant message {} vanished during update",
            message.id
        )));
    }
    Ok(AssistantMessage {
        meta_json: Some(meta),
        ..message.clone()
    })
}

/// Body of the trailing `Tool`-role message a confirmed action drops
/// into the transcript. `result_json` is a JSON object the UI can
/// pretty-print into the card body; `summary` is the human fallback.
fn build_tool_message(
    thread_id: AssistantThreadId,
    summary: String,
    result_json: serde_json::Value,
    created_at: codeless_types::UnixMillis,
) -> RpcResult<AssistantMessage> {
    let meta = serde_json::to_string(&result_json)
        .map_err(|e| RpcError::Internal(format!("serialise tool result: {e}")))?;
    Ok(AssistantMessage {
        id: AssistantMessageId::new(),
        thread_id,
        role: AssistantMessageRole::Tool,
        content: summary,
        meta_json: Some(meta),
        created_at,
    })
}

/// Dispatch the proposed action against the `RpcServer` surface and
/// fold the result into a `(summary, result_json)` pair. Errors flow
/// back up so the caller can record `Failed` rather than `Confirmed`
/// — the runtime never swallows the underlying RPC's typed error.
async fn dispatch_action(
    rpc: &InProcessRpc,
    action: &AssistantAction,
) -> RpcResult<(String, serde_json::Value)> {
    use serde_json::json;
    match action {
        AssistantAction::ListJobs { repo_id } => {
            let res = rpc.list_jobs(ListJobsArgs { repo_id: *repo_id }).await?;
            let summary = format!("Listed {} job(s).", res.jobs.len());
            Ok((summary, json!({ "tool": "list_jobs", "jobs": res.jobs })))
        }
        AssistantAction::GetJob { job_id } => {
            let job = rpc.get_job(GetJobArgs { job_id: *job_id }).await?;
            Ok((
                format!("Fetched job `{job_id}` (status: {:?}).", job.status),
                json!({ "tool": "get_job", "job": job }),
            ))
        }
        AssistantAction::StartJob { job_id } => {
            let job = rpc.start_job(StartJobArgs { job_id: *job_id }).await?;
            Ok((
                format!("Started job `{job_id}` (now {:?}).", job.status),
                json!({ "tool": "start_job", "job": job }),
            ))
        }
        AssistantAction::StopJob { job_id } => {
            rpc.stop_job(StopJobArgs { job_id: *job_id }).await?;
            Ok((
                format!("Stopped job `{job_id}`."),
                json!({ "tool": "stop_job", "job_id": job_id }),
            ))
        }
        AssistantAction::PauseJob { job_id } => {
            rpc.pause_job(PauseJobArgs { job_id: *job_id }).await?;
            Ok((
                format!("Paused job `{job_id}`."),
                json!({ "tool": "pause_job", "job_id": job_id }),
            ))
        }
        AssistantAction::ResumeJob { job_id } => {
            let job = rpc
                .resume_job(ResumeJobArgs {
                    job_id: *job_id,
                    additional_cost_cap_cents: None,
                    additional_wall_clock_cap_ms: None,
                })
                .await?;
            Ok((
                format!("Resumed job `{job_id}` (now {:?}).", job.status),
                json!({ "tool": "resume_job", "job": job }),
            ))
        }
        AssistantAction::RestartJob { job_id } => {
            let job = rpc
                .rerun_job(RerunJobArgs {
                    source_job_id: *job_id,
                })
                .await?;
            Ok((
                format!("Restarted job `{job_id}` as `{}`.", job.id),
                json!({ "tool": "restart_job", "job": job }),
            ))
        }
        AssistantAction::UpdateJob {
            job_id,
            runner,
            model,
            permission_mode,
            effort,
            cost_cap_cents,
            wall_clock_cap_ms,
            branch,
        } => {
            let job = rpc
                .update_job(UpdateJobArgs {
                    job_id: *job_id,
                    runner: runner.clone(),
                    model: model.clone(),
                    permission_mode: permission_mode.clone(),
                    effort: effort.clone(),
                    cost_cap_cents: *cost_cap_cents,
                    wall_clock_cap_ms: *wall_clock_cap_ms,
                    branch: branch.clone(),
                })
                .await?;
            Ok((
                format!("Updated job `{job_id}`."),
                json!({ "tool": "update_job", "job": job }),
            ))
        }
    }
}

pub(super) async fn confirm_assistant_action(
    rpc: &InProcessRpc,
    args: ConfirmAssistantActionArgs,
) -> RpcResult<ConfirmAssistantActionResult> {
    let (message, card) = load_pending_card(rpc, args.thread_id, args.message_id).await?;

    // Dispatch first, then write the resolved status so the persisted
    // state matches the outcome. A `Confirmed` row that points at no
    // tool message would otherwise be possible if a crash sat between
    // the status flip and the tool-message insert; running the RPC
    // first inverts that — a confirmed card always has a tool
    // companion, and a failure on the way appears as `Failed` plus a
    // tool message describing the error.
    let now = now_ms();
    match dispatch_action(rpc, &card.action).await {
        Ok((summary, result_json)) => {
            let card_row =
                write_card_status(rpc, &message, card, AssistantActionStatus::Confirmed).await?;
            let tool = build_tool_message(args.thread_id, summary, result_json, now)?;
            rpc.store
                .insert_assistant_message(&tool)
                .await
                .map_err(super::db_err)?;
            let _ = rpc
                .store
                .touch_assistant_thread(args.thread_id, now)
                .await
                .map_err(super::db_err)?;
            Ok(ConfirmAssistantActionResult {
                card: card_row,
                tool_message: tool,
            })
        }
        Err(err) => {
            let card_row =
                write_card_status(rpc, &message, card, AssistantActionStatus::Failed).await?;
            let summary = format!("Action failed: {err}");
            let tool = build_tool_message(
                args.thread_id,
                summary,
                serde_json::json!({ "tool": "error", "message": err.to_string() }),
                now,
            )?;
            rpc.store
                .insert_assistant_message(&tool)
                .await
                .map_err(super::db_err)?;
            let _ = rpc
                .store
                .touch_assistant_thread(args.thread_id, now)
                .await
                .map_err(super::db_err)?;
            Ok(ConfirmAssistantActionResult {
                card: card_row,
                tool_message: tool,
            })
        }
    }
}

pub(super) async fn cancel_assistant_action(
    rpc: &InProcessRpc,
    args: CancelAssistantActionArgs,
) -> RpcResult<CancelAssistantActionResult> {
    let (message, card) = load_pending_card(rpc, args.thread_id, args.message_id).await?;
    let card_row = write_card_status(rpc, &message, card, AssistantActionStatus::Cancelled).await?;
    let _ = rpc
        .store
        .touch_assistant_thread(args.thread_id, now_ms())
        .await
        .map_err(super::db_err)?;
    Ok(CancelAssistantActionResult { card: card_row })
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

    #[test]
    fn parser_recognises_each_slash_command() {
        let job_id = JobId::new();
        let line = format!("/start {job_id}");
        let (action, summary) = parse_action(&line).expect("parse start");
        assert!(matches!(action, AssistantAction::StartJob { job_id: j } if j == job_id));
        assert!(summary.contains("Start"));

        let (action, _) = parse_action("/list-jobs").expect("parse list");
        assert!(matches!(
            action,
            AssistantAction::ListJobs { repo_id: None }
        ));

        let (action, _) = parse_action(&format!("/restart {job_id}")).expect("parse restart");
        assert!(matches!(action, AssistantAction::RestartJob { .. }));

        let (action, summary) =
            parse_action(&format!("/update {job_id} model=claude-3 effort=high"))
                .expect("parse update");
        match action {
            AssistantAction::UpdateJob {
                model,
                effort,
                runner,
                ..
            } => {
                assert_eq!(model.as_deref(), Some("claude-3"));
                assert_eq!(effort.as_deref(), Some("high"));
                assert!(runner.is_none());
            }
            other => panic!("expected UpdateJob, got {other:?}"),
        }
        assert!(summary.contains("model"));
    }

    #[test]
    fn parser_returns_none_for_plain_text_and_bad_input() {
        assert!(parse_action("hi").is_none());
        assert!(parse_action("/start not-a-ulid").is_none());
        // `/update <id>` with no key=value pairs must not produce a
        // no-op patch — the parser folds back to the no-op responder
        // so the user gets the acknowledgement, not an empty card.
        let id = JobId::new();
        assert!(parse_action(&format!("/update {id}")).is_none());
        assert!(parse_action(&format!("/update {id} unknown_key=1")).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_with_slash_command_produces_pending_card() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let job_id = JobId::new();

        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/start {job_id}"),
            })
            .await
            .unwrap();
        let meta = res.assistant_message.meta_json.as_deref().expect("meta");
        let card: AssistantActionCard = serde_json::from_str(meta).expect("decode");
        assert_eq!(card.kind, AssistantActionCard::META_KIND);
        assert!(matches!(card.status, AssistantActionStatus::Pending));
        assert!(matches!(
            card.action,
            AssistantAction::StartJob { job_id: j } if j == job_id
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_marks_card_cancelled_and_appends_no_tool_message() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "/list-jobs".to_owned(),
            })
            .await
            .unwrap();
        let card_id = res.assistant_message.id;

        let cancel = rpc
            .cancel_assistant_action(CancelAssistantActionArgs {
                thread_id: thread.id,
                message_id: card_id,
            })
            .await
            .unwrap();
        let card: AssistantActionCard =
            serde_json::from_str(cancel.card.meta_json.as_deref().unwrap()).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Cancelled));

        // Transcript holds the user + assistant turn, no tool row.
        let listed = rpc
            .list_assistant_messages(ListAssistantMessagesArgs {
                thread_id: thread.id,
            })
            .await
            .unwrap()
            .messages;
        assert_eq!(listed.len(), 2);
        assert!(
            !listed
                .iter()
                .any(|m| matches!(m.role, AssistantMessageRole::Tool)),
            "no tool row should land for a cancelled card",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirm_dispatches_list_jobs_and_writes_tool_message() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "/list-jobs".into(),
            })
            .await
            .unwrap();
        let card_id = res.assistant_message.id;

        let confirm = rpc
            .confirm_assistant_action(ConfirmAssistantActionArgs {
                thread_id: thread.id,
                message_id: card_id,
            })
            .await
            .unwrap();
        let card: AssistantActionCard =
            serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Confirmed));
        assert!(matches!(
            confirm.tool_message.role,
            AssistantMessageRole::Tool
        ));
        assert!(confirm.tool_message.content.starts_with("Listed "));

        // Confirming the same card twice is rejected — status is no
        // longer pending, so the load helper bounces it.
        let err = rpc
            .confirm_assistant_action(ConfirmAssistantActionArgs {
                thread_id: thread.id,
                message_id: card_id,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirm_records_failed_card_when_inner_rpc_errors() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        // `/get <unknown>` will reach `get_job` and return NotFound.
        let phantom = JobId::new();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/get {phantom}"),
            })
            .await
            .unwrap();
        let confirm = rpc
            .confirm_assistant_action(ConfirmAssistantActionArgs {
                thread_id: thread.id,
                message_id: res.assistant_message.id,
            })
            .await
            .unwrap();
        let card: AssistantActionCard =
            serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Failed));
        assert!(confirm.tool_message.content.starts_with("Action failed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_unknown_message_is_not_found() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let err = rpc
            .cancel_assistant_action(CancelAssistantActionArgs {
                thread_id: thread.id,
                message_id: AssistantMessageId::new(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirm_rejects_non_card_message() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        // Plain chat — assistant turn has no meta_json.
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "hello".into(),
            })
            .await
            .unwrap();
        let err = rpc
            .confirm_assistant_action(ConfirmAssistantActionArgs {
                thread_id: thread.id,
                message_id: res.assistant_message.id,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }
}
