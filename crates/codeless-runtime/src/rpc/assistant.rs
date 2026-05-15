use codeless_rpc::{
    AppendAssistantMessageArgs, AppendAssistantMessageResult, CancelAssistantActionArgs,
    CancelAssistantActionResult, ConfirmAssistantActionArgs, ConfirmAssistantActionResult,
    CreateAssistantThreadArgs, DeleteAssistantThreadArgs, GetJobArgs, ListAssistantMessagesArgs,
    ListAssistantMessagesResult, ListAssistantThreadsArgs, ListAssistantThreadsResult,
    ListJobsArgs, PauseJobArgs, ReadJobFileArgs, RerunJobArgs, ResumeJobArgs, RpcError, RpcResult,
    RpcServer, StartJobArgs, StopJobArgs, SubmitJobArgs, UpdateJobArgs,
    UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult, WriteJobFileArgs,
};
use codeless_types::{
    AssistantAction, AssistantActionCard, AssistantActionStatus, AssistantAttachment,
    AssistantAttachmentId, AssistantMessage, AssistantMessageId, AssistantMessageRole,
    AssistantThread, AssistantThreadId, JobId, JobStatus, RepoId, WorkspaceMode,
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

/// Fallback used when the runtime is booted without an `agent_chat`
/// registry — typically tests and `codeless run --once`. The CLI's
/// `serve` path always wires the registry so the live product never
/// hits this branch; it exists so unit tests that don't care about
/// model dispatch keep round-tripping rows without spawning a fake.
const NOOP_ASSISTANT_REPLY: &str = "Assistant planner is not configured on this runtime; \
     boot with `with_agent_chat` to receive a model reply.";

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
    // Slash-command parser intercepts the typed action-card grammar;
    // anything else falls through to the F2 planner which streams a
    // model-generated reply. The planner is skipped (NOOP fallback) when
    // the runtime was booted without `with_agent_chat` so unit tests
    // that don't wire a fake registry still exercise the row plumbing.
    let (content, meta_json) = match parse_action(trimmed) {
        Some((action, summary)) => {
            let card = AssistantActionCard::new(action);
            let meta = serde_json::to_string(&card)
                .map_err(|e| RpcError::Internal(format!("serialise action card: {e}")))?;
            (summary, Some(meta))
        }
        None if super::assistant_planner::planner_configured(rpc) => {
            // History fold deliberately excludes the user row we just
            // inserted: the planner takes the new turn as its `Current
            // user message` trailer so the model treats it as the
            // message it is replying to, not as another historical entry.
            let history = rpc
                .store
                .list_assistant_messages(args.thread_id)
                .await
                .map_err(super::db_err)?;
            let prior: Vec<_> = history
                .into_iter()
                .filter(|m| m.id != user_message.id)
                .collect();
            let turn =
                super::assistant_planner::run_planner_turn(rpc, args.thread_id, &prior, trimmed)
                    .await?;
            (turn.content, None)
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
    // `/draft` is parsed up-front because its grammar is the only one
    // that carries a free-form prompt with whitespace; routing through
    // `split_whitespace` first would shred it. Every other branch is
    // whitespace-tokenised below and follows the same shape.
    if let Some(after) = rest
        .strip_prefix("draft ")
        .or_else(|| rest.strip_prefix("new "))
    {
        return parse_draft(after);
    }
    // `/edit-scope` carries a free-form body after `--`, same shape as
    // `/draft`. Routed before the whitespace-tokenised branches for the
    // same reason — the prompt would otherwise lose its embedded
    // newlines.
    if let Some(after) = rest
        .strip_prefix("edit-scope ")
        .or_else(|| rest.strip_prefix("scope "))
    {
        return parse_edit_scope(after);
    }
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

/// Stage-8 draft-from-conversation parser. Format:
///
/// ```text
/// /draft <repo_id> [key=value …] -- <prompt>
/// ```
///
/// Keys: `runner`, `branch`, `cost_cap_cents`, `wall_clock_cap_ms`,
/// `workspace_mode` (`in-repo` | `worktree`), `model`, `permission_mode`,
/// `effort`. Anything unrecognised fails the parse so the user sees the
/// no-op responder rather than a card with a silently-dropped knob.
///
/// Sensible defaults fill in for unprovided fields so a one-liner like
/// `/draft <repo_id> -- add dark mode` still produces a complete,
/// reviewable proposal. The defaults are surfaced on the card itself
/// (every field is stored, none implicit) so the confirmation is an
/// honest preview of what will be submitted.
fn parse_draft(after: &str) -> Option<(AssistantAction, String)> {
    let (head, prompt) = after.split_once("--")?;
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return None;
    }
    let mut toks = head.split_whitespace();
    let repo_id = toks.next()?.parse::<RepoId>().ok()?;

    let mut runner: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut cost_cap_cents: Option<i64> = None;
    let mut wall_clock_cap_ms: Option<i64> = None;
    let mut workspace_mode: Option<WorkspaceMode> = None;
    let mut model: Option<String> = None;
    let mut permission_mode: Option<String> = None;
    let mut effort: Option<String> = None;
    for tok in toks {
        let (k, v) = tok.split_once('=')?;
        match k {
            "runner" => runner = Some(v.to_owned()),
            "branch" => branch = Some(v.to_owned()),
            "cost_cap_cents" => cost_cap_cents = Some(v.parse().ok()?),
            "wall_clock_cap_ms" => wall_clock_cap_ms = Some(v.parse().ok()?),
            "workspace_mode" => {
                workspace_mode = Some(match v {
                    "in-repo" | "in_repo" => WorkspaceMode::InRepo,
                    "worktree" => WorkspaceMode::Worktree,
                    _ => return None,
                });
            }
            "model" => model = Some(v.to_owned()),
            "permission_mode" => permission_mode = Some(v.to_owned()),
            "effort" => effort = Some(v.to_owned()),
            _ => return None,
        }
    }

    let runner = runner.unwrap_or_else(|| "claude".to_owned());
    let branch = branch.unwrap_or_else(|| "assistant/draft".to_owned());
    // 30-minute / $5.00 caps as a placeholder that errs on the side of
    // "small enough to be safe to confirm". Real planners override.
    let cost_cap_cents = cost_cap_cents.unwrap_or(500);
    let wall_clock_cap_ms = wall_clock_cap_ms.unwrap_or(30 * 60 * 1000);

    let summary = format!(
        "Draft job in repo `{repo_id}` on branch `{branch}` \
         (runner `{runner}`, caps {cost_cap_cents}¢ / {wall_clock_cap_ms}ms).\n\n\
         Prompt:\n{prompt}",
    );
    Some((
        AssistantAction::DraftJob {
            repo_id,
            prompt,
            runner,
            branch,
            cost_cap_cents,
            wall_clock_cap_ms,
            workspace_mode,
            model,
            permission_mode,
            effort,
        },
        summary,
    ))
}

/// Stage-9 edit-scope parser. Format:
///
/// ```text
/// /edit-scope <job_id> [filename=SCOPE.md] -- <new body>
/// ```
///
/// `filename` defaults to `SCOPE.md` — the common case is the user
/// rewriting the high-level brief. `WORKFLOW.md` and other non-
/// template files are reachable through the optional key. The body
/// after `--` is taken verbatim (whitespace preserved); an empty body
/// falls back to the no-op responder rather than emitting a card
/// that would silently clobber the file with nothing.
fn parse_edit_scope(after: &str) -> Option<(AssistantAction, String)> {
    let (head, body) = after.split_once("--")?;
    // Strip the single space (or newline) that typically follows the
    // `--` separator. Trailing whitespace is preserved verbatim so the
    // user can end the file with a deliberate blank line — a markdown
    // convention some downstream renderers care about.
    let new_content = body.trim_start().to_owned();
    if new_content.is_empty() {
        return None;
    }
    let mut toks = head.split_whitespace();
    let job_id = toks.next()?.parse::<JobId>().ok()?;
    let mut filename: Option<String> = None;
    for tok in toks {
        let (k, v) = tok.split_once('=')?;
        match k {
            "filename" | "file" => filename = Some(v.to_owned()),
            _ => return None,
        }
    }
    let filename = filename.unwrap_or_else(|| "SCOPE.md".to_owned());
    let summary = format!(
        "Edit `{filename}` on job `{job_id}` ({} bytes proposed). \
         The card carries the full new body; confirm to write through \
         `write_job_file` once the job is non-running.",
        new_content.len(),
    );
    Some((
        AssistantAction::EditScope {
            job_id,
            filename,
            new_content,
        },
        summary,
    ))
}

/// Tiny LCS-based unified diff over lines. Good enough for the card's
/// "preview what `write_job_file` will land" — the runtime never
/// promises this is byte-identical to `git diff` output. A real diff
/// library would be heavier than the carry; the algorithm here is the
/// textbook DP table walked twice (forward to fill, backward to emit).
///
/// Lines keep their newlines; the emitted `String` is plain
/// `-`/`+`/` ` prefixed text with a synthetic `@@` header so the UI
/// can render it inside a `<pre>` without parsing.
fn unified_diff(old: &str, new: &str, label: &str) -> String {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = format!("--- {label} (current)\n+++ {label} (proposed)\n");
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(' ');
            out.push_str(a[i]);
            if !a[i].ends_with('\n') {
                out.push('\n');
            }
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push('-');
            out.push_str(a[i]);
            if !a[i].ends_with('\n') {
                out.push('\n');
            }
            i += 1;
        } else {
            out.push('+');
            out.push_str(b[j]);
            if !b[j].ends_with('\n') {
                out.push('\n');
            }
            j += 1;
        }
    }
    while i < n {
        out.push('-');
        out.push_str(a[i]);
        if !a[i].ends_with('\n') {
            out.push('\n');
        }
        i += 1;
    }
    while j < m {
        out.push('+');
        out.push_str(b[j]);
        if !b[j].ends_with('\n') {
            out.push('\n');
        }
        j += 1;
    }
    out
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
        AssistantAction::DraftJob {
            repo_id,
            prompt,
            runner,
            branch,
            cost_cap_cents,
            wall_clock_cap_ms,
            workspace_mode,
            model,
            permission_mode,
            effort,
        } => {
            // SCOPE.md Decisions §3 — no "just do it" path. The card
            // is the confirmation; landing the row as `Draft` lets the
            // user edit the spec / docs / handover before kicking off
            // the runner, matching the regular submit-from-CLI flow.
            let job = rpc
                .submit_job(SubmitJobArgs {
                    repo_id: *repo_id,
                    prompt: Some(prompt.clone()),
                    template_yaml: None,
                    runner: runner.clone(),
                    branch: branch.clone(),
                    workspace_mode: *workspace_mode,
                    cost_cap_cents: *cost_cap_cents,
                    wall_clock_cap_ms: *wall_clock_cap_ms,
                    model: model.clone(),
                    permission_mode: permission_mode.clone(),
                    effort: effort.clone(),
                    start_immediately: false,
                })
                .await?;
            Ok((
                format!("Drafted job `{}` (status: {:?}).", job.id, job.status),
                json!({ "tool": "draft_job", "job": job }),
            ))
        }
        AssistantAction::EditScope {
            job_id,
            filename,
            new_content,
        } => {
            // The paused-job rule: a chat-driven spec edit must not
            // race the runner that is currently reading the same file
            // off disk. Only non-running statuses go through; the user
            // pauses (or stops, or waits for the job to finish) and
            // re-confirms. `update_job_template` does not enforce this
            // because the CLI surface accepts the risk of editing a
            // live job; the assistant surface is the friendlier path
            // and the gate lives here.
            let job = rpc.get_job(GetJobArgs { job_id: *job_id }).await?;
            if matches!(
                job.status,
                JobStatus::Running | JobStatus::Queued | JobStatus::AwaitingReview
            ) {
                return Err(RpcError::Conflict(format!(
                    "job {job_id} is {:?}; pause it first with `/pause {job_id}` \
                     before editing the spec",
                    job.status
                )));
            }

            // Read the current body so we can emit a diff in the
            // tool message. NotFound means the file does not exist
            // yet — treat that as an empty current body so the diff
            // shows the whole new content as additions, matching
            // what `git diff` would say about a brand-new file.
            let current = match rpc
                .read_job_file(ReadJobFileArgs {
                    job_id: *job_id,
                    filename: filename.clone(),
                })
                .await
            {
                Ok(res) => res.content,
                Err(RpcError::NotFound(_)) => String::new(),
                Err(e) => return Err(e),
            };

            let result = rpc
                .write_job_file(WriteJobFileArgs {
                    job_id: *job_id,
                    filename: filename.clone(),
                    content: new_content.clone(),
                })
                .await?;
            let diff = unified_diff(&current, new_content, &result.name);
            Ok((
                format!(
                    "Wrote `{}` on job `{job_id}` ({} → {} bytes).",
                    result.name,
                    current.len(),
                    new_content.len(),
                ),
                json!({
                    "tool": "edit_scope",
                    "job_id": job_id,
                    "filename": result.name,
                    "diff": diff,
                }),
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
    async fn append_with_planner_persists_streamed_reply_with_empty_meta() {
        use super::super::assistant_planner::tests::FakeChatRunner;
        use std::sync::Arc;

        let runner = Arc::new(FakeChatRunner::new(vec!["streamed ", "reply"]));
        let registry = ai_runner::Registry::new();
        registry.register(runner);
        let rpc = InProcessRpc::new()
            .await
            .unwrap()
            .with_agent_chat(Arc::new(registry), std::env::temp_dir());
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();

        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: "what jobs are running?".into(),
            })
            .await
            .unwrap();

        assert_eq!(res.user_message.role, AssistantMessageRole::User);
        assert_eq!(res.assistant_message.role, AssistantMessageRole::Assistant);
        assert_eq!(res.assistant_message.content, "streamed reply");
        // F2 chat replies are not action cards: meta_json must be NULL
        // so the UI's CommonChat renderer treats the row as plain prose.
        assert!(res.assistant_message.meta_json.is_none());

        // Slash commands keep their action-card path even with the
        // planner wired — `meta_json` must carry the card payload.
        let job_id = JobId::new();
        let card_res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/start {job_id}"),
            })
            .await
            .unwrap();
        assert!(card_res.assistant_message.meta_json.is_some());
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

    #[test]
    fn parser_draft_extracts_prompt_and_defaults() {
        let repo_id = RepoId::new();
        let line = format!("/draft {repo_id} -- add dark mode");
        let (action, summary) = parse_action(&line).expect("parse draft");
        match action {
            AssistantAction::DraftJob {
                repo_id: r,
                prompt,
                runner,
                branch,
                cost_cap_cents,
                wall_clock_cap_ms,
                workspace_mode,
                model,
                permission_mode,
                effort,
            } => {
                assert_eq!(r, repo_id);
                assert_eq!(prompt, "add dark mode");
                // Defaults are surfaced explicitly on the card so the
                // confirmation preview is honest about what gets submitted.
                assert_eq!(runner, "claude");
                assert_eq!(branch, "assistant/draft");
                assert_eq!(cost_cap_cents, 500);
                assert_eq!(wall_clock_cap_ms, 30 * 60 * 1000);
                assert!(workspace_mode.is_none());
                assert!(model.is_none());
                assert!(permission_mode.is_none());
                assert!(effort.is_none());
            }
            other => panic!("expected DraftJob, got {other:?}"),
        }
        assert!(summary.contains("Draft job"));
        assert!(summary.contains("add dark mode"));
    }

    #[test]
    fn parser_draft_honours_overrides() {
        let repo_id = RepoId::new();
        let line = format!(
            "/draft {repo_id} runner=copilot branch=feat/x cost_cap_cents=1234 \
             wall_clock_cap_ms=99 workspace_mode=worktree model=gpt-5 \
             permission_mode=plan effort=high -- do the thing"
        );
        let (action, _) = parse_action(&line).expect("parse draft");
        match action {
            AssistantAction::DraftJob {
                runner,
                branch,
                cost_cap_cents,
                wall_clock_cap_ms,
                workspace_mode,
                model,
                permission_mode,
                effort,
                prompt,
                ..
            } => {
                assert_eq!(runner, "copilot");
                assert_eq!(branch, "feat/x");
                assert_eq!(cost_cap_cents, 1234);
                assert_eq!(wall_clock_cap_ms, 99);
                assert!(matches!(workspace_mode, Some(WorkspaceMode::Worktree)));
                assert_eq!(model.as_deref(), Some("gpt-5"));
                assert_eq!(permission_mode.as_deref(), Some("plan"));
                assert_eq!(effort.as_deref(), Some("high"));
                assert_eq!(prompt, "do the thing");
            }
            other => panic!("expected DraftJob, got {other:?}"),
        }
    }

    #[test]
    fn parser_draft_rejects_missing_prompt_or_unknown_key() {
        let repo_id = RepoId::new();
        // Missing `--` separator → no prompt → no card.
        assert!(parse_action(&format!("/draft {repo_id} add dark mode")).is_none());
        // Empty prompt after `--` likewise drops to the no-op reply.
        assert!(parse_action(&format!("/draft {repo_id} --   ")).is_none());
        // Unknown key fails the parse rather than silently dropping the field.
        assert!(parse_action(&format!("/draft {repo_id} weird=1 -- p")).is_none());
        // Bad repo id → unparseable, no card.
        assert!(parse_action("/draft not-a-ulid -- p").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_with_draft_command_emits_pending_card() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let repo_id = RepoId::new();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/draft {repo_id} -- ship it"),
            })
            .await
            .unwrap();
        let card: AssistantActionCard =
            serde_json::from_str(res.assistant_message.meta_json.as_deref().unwrap()).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Pending));
        assert!(matches!(
            card.action,
            AssistantAction::DraftJob { repo_id: r, .. } if r == repo_id
        ));
        // The mutates() guard is the only place that knows blast radius;
        // a new variant flowing through without an arm in the match is a
        // compile error, but the bool itself is what the UI keys off of.
        assert!(card.action.mutates());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirm_draft_against_unknown_repo_records_failed() {
        // submit_job hits the repo lookup before any filesystem work,
        // so a phantom repo_id surfaces as a typed NotFound from the
        // inner RPC. The confirm path turns that into status=Failed plus
        // a Tool message describing the error — the contract we want
        // the UI to be able to rely on.
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let phantom_repo = RepoId::new();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/draft {phantom_repo} -- ship it"),
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
        assert!(matches!(
            confirm.tool_message.role,
            AssistantMessageRole::Tool
        ));
        assert!(confirm.tool_message.content.starts_with("Action failed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_draft_card_writes_no_job() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let repo_id = RepoId::new();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/draft {repo_id} -- nope"),
            })
            .await
            .unwrap();
        let cancel = rpc
            .cancel_assistant_action(CancelAssistantActionArgs {
                thread_id: thread.id,
                message_id: res.assistant_message.id,
            })
            .await
            .unwrap();
        let card: AssistantActionCard =
            serde_json::from_str(cancel.card.meta_json.as_deref().unwrap()).unwrap();
        assert!(matches!(card.status, AssistantActionStatus::Cancelled));
        // Cancel must not have leaked a job row — the rejected proposal
        // is recorded in the transcript and nothing else.
        let jobs = rpc.list_jobs(ListJobsArgs { repo_id: None }).await.unwrap();
        assert!(jobs.jobs.is_empty(), "no job should land on cancel");
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

    #[test]
    fn parser_edit_scope_extracts_defaults_and_body() {
        let job_id = JobId::new();
        let line = format!("/edit-scope {job_id} -- # new scope\n\nlots of words");
        let (action, summary) = parse_action(&line).expect("parse edit-scope");
        match action {
            AssistantAction::EditScope {
                job_id: j,
                filename,
                new_content,
            } => {
                assert_eq!(j, job_id);
                assert_eq!(filename, "SCOPE.md");
                // Embedded newlines survive the parser — the body is taken
                // verbatim past the `--` separator so multi-line spec
                // rewrites round-trip without escaping.
                assert!(new_content.starts_with("# new scope"));
                assert!(new_content.contains("lots of words"));
            }
            other => panic!("expected EditScope, got {other:?}"),
        }
        assert!(summary.contains("SCOPE.md"));
    }

    #[test]
    fn parser_edit_scope_honours_filename_override() {
        let job_id = JobId::new();
        let (action, _) = parse_action(&format!(
            "/edit-scope {job_id} filename=WORKFLOW.md -- body"
        ))
        .expect("parse edit-scope");
        match action {
            AssistantAction::EditScope { filename, .. } => assert_eq!(filename, "WORKFLOW.md"),
            other => panic!("expected EditScope, got {other:?}"),
        }
        // `/scope` alias mirrors the `/new` ↔ `/draft` shape so the user can
        // shorthand the common path.
        assert!(parse_action(&format!("/scope {job_id} -- body")).is_some());
    }

    #[test]
    fn parser_edit_scope_rejects_missing_body_or_unknown_key() {
        let job_id = JobId::new();
        assert!(parse_action(&format!("/edit-scope {job_id} new content")).is_none());
        assert!(parse_action(&format!("/edit-scope {job_id} --   \n  \t")).is_none());
        assert!(parse_action(&format!("/edit-scope {job_id} weird=1 -- body")).is_none());
        assert!(parse_action("/edit-scope not-a-ulid -- body").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn confirm_edit_scope_unknown_job_records_failed() {
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();
        let phantom = JobId::new();
        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/edit-scope {phantom} -- new body"),
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
    async fn confirm_edit_scope_against_running_job_is_refused() {
        // The paused-job rule lives on the dispatch arm, not on
        // `write_job_file`. The CLI surface for `write_job_file`
        // accepts the risk of editing a live job; the chat surface
        // refuses so the runner does not race the user's spec rewrite.
        // We can prove the guard fires without touching git by sitting
        // the job at Running before the dispatch ever reaches
        // `read_job_file` / `write_job_file`.
        use codeless_types::{CostCents, GitAuth, Job, Repo};
        let (rpc, _data) = rpc_with_data_dir().await;
        let thread = rpc
            .create_assistant_thread(CreateAssistantThreadArgs { title: None })
            .await
            .unwrap();

        let repo_id = RepoId::new();
        let now = now_ms();
        rpc.store
            .insert_repo(&Repo {
                id: repo_id,
                name: "test".into(),
                clone_url: "ssh://x/y".into(),
                default_branch: "main".into(),
                // A non-existent path is fine: the Running guard short-
                // circuits before any filesystem reach.
                local_path: "/nonexistent/codeless-test-repo".into(),
                git_auth: GitAuth::Token {
                    env_var: "TOKEN".into(),
                },
                concurrency_cap: None,
                default_runner: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let job_id = JobId::new();
        rpc.store
            .insert_job(&Job {
                id: job_id,
                repo_id,
                status: JobStatus::Running,
                stop_reason: None,
                template_yaml: None,
                prompt: Some("noop".into()),
                runner: "claude".into(),
                branch: "main".into(),
                workspace_mode: WorkspaceMode::InRepo,
                worktree_path: None,
                cost_cap_cents: CostCents(100),
                wall_clock_cap_ms: 1000,
                cost_cents: CostCents::ZERO,
                model: None,
                permission_mode: None,
                effort: None,
                started_at: Some(now),
                ended_at: None,
                created_at: now,
            })
            .await
            .unwrap();

        let res = rpc
            .append_assistant_message(AppendAssistantMessageArgs {
                thread_id: thread.id,
                content: format!("/edit-scope {job_id} -- # rewritten"),
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
        // The tool message is the user-readable summary of the typed
        // Conflict — it must mention the paused-job remedy so the user
        // knows what to do next.
        assert!(
            confirm.tool_message.content.contains("pause"),
            "tool message should mention pause: {}",
            confirm.tool_message.content,
        );
    }

    #[test]
    fn unified_diff_marks_additions_and_deletions() {
        let old = "one\ntwo\nthree\n";
        let new = "one\nTWO\nthree\nfour\n";
        let diff = unified_diff(old, new, "SCOPE.md");
        assert!(diff.contains("--- SCOPE.md (current)"));
        assert!(diff.contains("+++ SCOPE.md (proposed)"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+TWO"));
        assert!(diff.contains("+four"));
        // A pure addition (empty current) renders every line as `+`.
        // Stripping the two-line synthetic header keeps the assertion
        // honest — the header itself starts with `---` / `+++`, which
        // is part of the diff format, not a deletion.
        let only_adds = unified_diff("", "a\nb\n", "SCOPE.md");
        let body: String = only_adds.lines().skip(2).collect::<Vec<_>>().join("\n");
        assert!(body.contains("+a"));
        assert!(body.contains("+b"));
        assert!(!body.contains('-'));
    }
}
