use std::sync::Arc;

use codeless_rpc::{
    AgentChatArgs, AgentChatResult, CancelChatTaskArgs, ChatMode, JobContextRef, JobReportArgs,
    RpcError, RpcResult, StopActiveArgs, StopActiveResult, StopJobArgs, UploadChatAttachmentArgs,
    UploadChatAttachmentResult,
};
use codeless_types::{RepoId, TaskId};

use super::{ChatCancelEntry, ChatCancels, InProcessRpc};
use crate::template::JobTemplate;
use crate::time::now_ms;

/// RAII guard that removes a chat-cancel entry when the spawned chat
/// task ends. Held across the `run_chat` future so success, error, and
/// panic all evict the token; without this the registry would leak
/// entries every time a turn completes naturally.
struct ChatCancelGuard {
    cancels: ChatCancels,
    task_id: TaskId,
}

impl Drop for ChatCancelGuard {
    fn drop(&mut self) {
        self.cancels.lock().remove(&self.task_id);
    }
}

/// Per-process counter to disambiguate attachment uploads that land in
/// the same millisecond. Wraps cheaply; the millis prefix ensures
/// uniqueness in practice.
static ATTACHMENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) async fn agent_chat(
    rpc: &InProcessRpc,
    args: AgentChatArgs,
) -> RpcResult<AgentChatResult> {
    let registry = rpc.agent_chat_registry.as_ref().ok_or_else(|| {
        RpcError::Internal("agent_chat registry is not configured on this runtime".to_owned())
    })?;
    let default_cwd = rpc.agent_chat_cwd.clone().ok_or_else(|| {
        RpcError::Internal("agent_chat cwd is not configured on this runtime".to_owned())
    })?;
    // Per-call cwd override for the per-job chat panel. Reject paths
    // outside the configured fs roots rather than silently falling back.
    let cwd = match args.cwd.as_deref() {
        Some(p) => {
            let abs = std::path::PathBuf::from(p);
            let canon = std::fs::canonicalize(&abs).map_err(|_| {
                RpcError::InvalidArgument(format!("agent_chat cwd does not exist: {p}"))
            })?;
            if !canon.is_dir() {
                return Err(RpcError::InvalidArgument(format!(
                    "agent_chat cwd is not a directory: {p}"
                )));
            }
            let fs_allowed = rpc
                .fs
                .as_ref()
                .map(|fs| fs.is_path_allowed(&canon))
                .unwrap_or(false);
            // Also allow cwd under any registered repo's local_path so
            // the per-job chat panel can target repos outside --fs-root.
            let repo_allowed = if !fs_allowed {
                let repos = rpc.store.list_repos().await.map_err(super::db_err)?;
                repos.iter().any(|r| {
                    std::fs::canonicalize(&r.local_path)
                        .map(|rp| canon.starts_with(&rp))
                        .unwrap_or(false)
                })
            } else {
                false
            };
            if !fs_allowed && !repo_allowed {
                return Err(RpcError::InvalidArgument(format!(
                    "agent_chat cwd is outside the configured fs roots: {p}"
                )));
            }
            canon
        }
        None => default_cwd,
    };
    let provider = codeless_adapters_host::parse_cli_runner_id(&args.runner).ok_or_else(|| {
        RpcError::InvalidArgument(format!("unknown CLI runner id `{}`", args.runner))
    })?;

    let session_id = args.session_id;
    let task_id = codeless_types::TaskId::new();
    let bus = Arc::clone(&rpc.bus);
    let registry = Arc::clone(registry);
    // When the chat session_id maps to a real job, fold that job's spec
    // files into the preamble. Footer-panel turns pass a fresh
    // correlation id that does not resolve to a job — both cases skip
    // the block silently.
    let mode = args.mode.unwrap_or_default();
    let active_job = rpc.store.get_job(session_id).await.ok().flatten();
    let job_spec_block = match &active_job {
        Some(job) => load_chat_job_spec_for(rpc, job).await,
        None => None,
    };
    let job_refs_block = load_chat_job_refs(
        rpc,
        active_job.as_ref().map(|j| j.repo_id),
        args.context
            .as_ref()
            .map(|c| c.job_refs.as_slice())
            .unwrap_or(&[]),
    )
    .await?;
    let prompt = build_chat_prompt(
        args.context.as_ref(),
        job_spec_block.as_deref(),
        job_refs_block.as_deref(),
        mode,
        &args.prompt,
    );
    // Spec mode: clamp the agent to read + edit tools so it can author
    // the job spec but cannot run Bash, hit the network, or git commit.
    // `tools` maps to `--tools` on the claude binary which restricts the
    // available built-in tool set — not `--allowed-tools` which is for
    // MCP server permissions only and does not block Bash.
    let tools = match mode {
        ChatMode::Spec => Some(SPEC_MODE_ALLOWED_TOOLS.to_owned()),
        ChatMode::Work => None,
    };

    // Register the cancel token before the spawn so a racing
    // `cancel_chat_task` still finds an entry to fire. The drop-guard
    // inside the task removes the entry on any exit.
    let cancel = tokio_util::sync::CancellationToken::new();
    rpc.chat_cancels.lock().insert(
        task_id,
        ChatCancelEntry {
            job_id: session_id,
            token: cancel.clone(),
        },
    );
    let cancels = Arc::clone(&rpc.chat_cancels);

    // Detached: the call returns once the runner has been spawned; its
    // tokens/tool-calls/completion events flow back through the bus.
    tokio::spawn(async move {
        let _guard = ChatCancelGuard { cancels, task_id };
        let publish = move |event: codeless_types::Event| {
            let bus = Arc::clone(&bus);
            async move {
                bus.publish(Some(session_id), None, Some(task_id), event, now_ms())
                    .await
                    .map(|_| ())
            }
        };
        if let Err(e) = codeless_adapters_host::run_chat(
            registry,
            codeless_adapters_host::ChatRunCfg {
                provider,
                prompt,
                cwd,
                tools,
            },
            task_id,
            publish,
            cancel,
        )
        .await
        {
            tracing::warn!(error = %e, "agent_chat run failed");
        }
    });

    Ok(AgentChatResult {
        session_id,
        task_id,
    })
}

pub(super) async fn upload_chat_attachment(
    rpc: &InProcessRpc,
    args: UploadChatAttachmentArgs,
) -> RpcResult<UploadChatAttachmentResult> {
    use base64::Engine as _;

    // Worktree-scoped: attachments under `.codeless/chat-attachments/`
    // are reachable by relative path from the runner's cwd. Conflict if
    // no worktree has been provisioned yet — there's no sensible fallback.
    let job = rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
    let worktree = job.worktree_path.as_deref().ok_or_else(|| {
        RpcError::Conflict(format!(
            "job {} has no worktree yet; submit/run the job before attaching files",
            args.job_id
        ))
    })?;

    // Reuse the job-file sanitiser for path traversal / dotfile rejection.
    let safe = crate::job_dir::sanitise_filename(&args.filename)
        .map_err(|e| RpcError::InvalidArgument(format!("filename: {e:?}")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(args.content_b64.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(args.content_b64.as_bytes())
        })
        .map_err(|e| RpcError::InvalidArgument(format!("content_b64: {e}")))?;

    let dir = std::path::Path::new(worktree)
        .join(".codeless")
        .join("chat-attachments");
    std::fs::create_dir_all(&dir).map_err(|e| {
        RpcError::Internal(format!(
            "create chat-attachments dir {}: {e}",
            dir.display()
        ))
    })?;

    // Unique prefix: millis + per-process atomic counter.
    let stamp = now_ms().0;
    let seq = ATTACHMENT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stored = format!("{stamp}-{seq}-{safe}");
    let abs = dir.join(&stored);
    std::fs::write(&abs, &bytes)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", abs.display())))?;

    let relative_path = format!(".codeless/chat-attachments/{stored}");
    Ok(UploadChatAttachmentResult {
        relative_path,
        absolute_path: abs.to_string_lossy().into_owned(),
    })
}

pub(super) async fn cancel_chat_task(
    rpc: &InProcessRpc,
    args: CancelChatTaskArgs,
) -> RpcResult<()> {
    // Idempotent: a missing entry means the turn already completed or
    // was cancelled by a previous call. `Ok(())` lets the UI race the
    // natural end of the stream without distinguishing "stopped" from
    // "already over".
    if let Some(entry) = rpc.chat_cancels.lock().get(&args.task_id) {
        entry.token.cancel();
    }
    Ok(())
}

pub(super) async fn stop_active(
    rpc: &InProcessRpc,
    args: StopActiveArgs,
) -> RpcResult<StopActiveResult> {
    // Job side: only call `stop_job` when the row is in a state it
    // accepts. The match must mirror the guard in `stop_job`.
    let stopped_job = match rpc
        .store
        .get_job(args.job_id)
        .await
        .map_err(super::db_err)?
    {
        Some(job)
            if matches!(
                job.status,
                codeless_types::JobStatus::Running
                    | codeless_types::JobStatus::AwaitingReview
                    | codeless_types::JobStatus::Queued
                    | codeless_types::JobStatus::Paused
                    | codeless_types::JobStatus::Draft
            ) =>
        {
            super::jobs::stop_job(
                rpc,
                StopJobArgs {
                    job_id: args.job_id,
                },
            )
            .await?;
            true
        }
        Some(_) => false,
        None => return Err(RpcError::NotFound(format!("job {}", args.job_id))),
    };

    // Chat side: snapshot matching entries under the lock, fire tokens
    // outside it. Drop-guards evict entries themselves; we leave them in
    // place so a racing second `stop_active` is a no-op fire.
    let cancelled_chat_task_ids: Vec<TaskId> = {
        let map = rpc.chat_cancels.lock();
        map.iter()
            .filter(|(_, entry)| entry.job_id == args.job_id)
            .map(|(task_id, entry)| {
                entry.token.cancel();
                *task_id
            })
            .collect()
    };

    Ok(StopActiveResult {
        stopped_job,
        cancelled_chat_task_ids,
    })
}

/// Best-effort fetch of the active job's spec for the chat preamble.
/// Returns `None` when the job lacks a parseable template or when none
/// of the spec files are present. Files are bounded by
/// `MAX_CHAT_SPEC_BYTES` per file so a runaway SCOPE.md cannot blow
/// out the model's context budget.
pub(super) async fn load_chat_job_spec_for(
    rpc: &InProcessRpc,
    job: &codeless_types::Job,
) -> Option<String> {
    let template_yaml = job.template_yaml.as_ref()?;
    let template = JobTemplate::parse_yaml(template_yaml).ok()?;
    let repo = rpc.store.get_repo(job.repo_id).await.ok().flatten()?;
    let mut out = render_job_spec_section(job, &template, template_yaml, &repo.local_path, true);
    out.push_str(CHAT_JOB_SPEC_AUTHORING_PRIMER);
    Some(out)
}

/// Render the per-ref preamble fold listing every referenced job the
/// caller opted into. Returns `Ok(None)` for the trivial empty-refs
/// case so the preamble shape stays unchanged when nothing is attached.
///
/// Cross-repo refs and footer-panel chats (no active job → no repo to
/// gate against) are rejected with `InvalidArgument` rather than
/// silently skipped: the UI restricts the picker to same-repo jobs and
/// raw-prompt callers should not be sending `job_refs` at all.
pub(super) async fn load_chat_job_refs(
    rpc: &InProcessRpc,
    active_repo_id: Option<RepoId>,
    refs: &[JobContextRef],
) -> RpcResult<Option<String>> {
    if refs.is_empty() {
        return Ok(None);
    }
    let active_repo_id = active_repo_id.ok_or_else(|| {
        RpcError::InvalidArgument("job_refs require chat to be scoped to a real job".to_owned())
    })?;

    let mut out = String::from("## Referenced jobs\n\n");
    for r in refs {
        let job = rpc
            .store
            .get_job(r.job_id)
            .await
            .map_err(super::db_err)?
            .ok_or_else(|| {
                RpcError::InvalidArgument(format!("referenced job {} not found", r.job_id))
            })?;
        if job.repo_id != active_repo_id {
            return Err(RpcError::InvalidArgument(format!(
                "referenced job {} is in a different repo from the active job",
                r.job_id
            )));
        }
        let template_yaml = job.template_yaml.as_deref().unwrap_or("");
        let template_name = JobTemplate::parse_yaml(template_yaml)
            .ok()
            .map(|t| t.name)
            .unwrap_or_else(|| format!("job-{}", r.job_id));

        out.push_str(&format!(
            "### {} (id `{}`, status `{:?}`)\n\n",
            template_name, r.job_id, job.status,
        ));

        if r.include_spec {
            if let Some(repo) = rpc.store.get_repo(job.repo_id).await.ok().flatten() {
                if let Ok(template) = JobTemplate::parse_yaml(template_yaml) {
                    let section = render_job_spec_section(
                        &job,
                        &template,
                        template_yaml,
                        &repo.local_path,
                        false,
                    );
                    out.push_str(&section);
                } else {
                    out.push_str("_(spec unavailable: job has no parseable template)_\n\n");
                }
            }
        }

        if r.include_history {
            let report = super::jobs::job_report(rpc, JobReportArgs { job_id: r.job_id }).await?;
            let limit = r
                .history_turn_limit
                .unwrap_or(DEFAULT_REF_HISTORY_TURN_LIMIT) as usize;
            let history = render_ref_history(&report, limit);
            out.push_str(&truncate_with_cap(&history, MAX_CHAT_HISTORY_BYTES));
            if !history.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    }
    Ok(Some(out))
}

/// Render the spec-files section (template.yaml + SCOPE + WORKFLOW)
/// for one job. Shared between the active-job fold and per-ref folds.
/// `is_active` flips the framing line ("Active job…" vs "Spec for
/// reference…") and otherwise the layout is identical so the model
/// sees a familiar shape regardless of which fold sourced it.
fn render_job_spec_section(
    job: &codeless_types::Job,
    template: &JobTemplate,
    template_yaml: &str,
    repo_local_path: &str,
    is_active: bool,
) -> String {
    let job_dir = std::path::Path::new(repo_local_path)
        .join(".codeless")
        .join("jobs")
        .join(&template.name);

    let mut out = String::new();
    if is_active {
        out.push_str(&format!(
            "Active job: {} (id `{}`, status `{:?}`).\n\
             Spec lives at `.codeless/jobs/{}/` in the repo. The files \
             reproduced below are the source of truth for this job; \
             prefer them over anything else when answering.\n\n",
            template.name, job.id, job.status, template.name,
        ));
    } else {
        out.push_str(&format!(
            "Spec lives at `.codeless/jobs/{}/` in the same repo.\n\n",
            template.name,
        ));
    }
    out.push_str("#### template.yaml\n\n```yaml\n");
    out.push_str(&truncate_for_chat(template_yaml));
    if !template_yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");

    for (label, filename) in [("SCOPE.md", "SCOPE.md"), ("WORKFLOW.md", "WORKFLOW.md")] {
        let path = job_dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            out.push_str(&format!("#### {label}\n\n"));
            out.push_str(&truncate_for_chat(&content));
            if !content.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    }
    out
}

/// Render a snapshot of one referenced job's recent activity from a
/// `job_report` result. Bounded by the caller through
/// `truncate_with_cap`; the limit on turn count is applied here so we
/// don't waste budget formatting rows that will be cut.
fn render_ref_history(report: &codeless_rpc::JobReportResult, turn_limit: usize) -> String {
    let mut out = String::from("#### Recent activity\n\n");
    out.push_str(&format!(
        "status `{}`, cost {}c of {}c cap.\n\n",
        report.status, report.cost_cents, report.cost_cap_cents,
    ));

    if !report.stages.is_empty() {
        out.push_str("Stages:\n");
        for s in &report.stages {
            out.push_str(&format!(
                "- #{}.{} `{}` — {} (cost {}c{})\n",
                s.ordinal,
                s.attempt,
                s.title,
                s.status,
                s.cost_cents,
                match s.duration_ms {
                    Some(ms) => format!(", {}ms", ms),
                    None => String::new(),
                },
            ));
        }
        out.push('\n');
    }

    let turn_count = report.turns.len();
    let start = turn_count.saturating_sub(turn_limit);
    if turn_count > 0 {
        out.push_str(&format!(
            "Last {} of {} turns:\n",
            turn_count - start,
            turn_count,
        ));
        for t in &report.turns[start..] {
            out.push_str(&format!(
                "- task `{}`{} — cost {}c, {} in / {} out tokens\n",
                t.task_id,
                match t.stage_ordinal {
                    Some(o) => format!(" (stage {})", o),
                    None => String::new(),
                },
                t.cost_cents,
                t.input_tokens,
                t.output_tokens,
            ));
        }
        out.push('\n');
    }

    if !report.tool_calls.is_empty() {
        out.push_str("Tool calls:\n");
        for tc in &report.tool_calls {
            out.push_str(&format!("- {} ×{}\n", tc.tool, tc.count));
        }
        out.push('\n');
    }
    out
}

/// Render the optional `ChatContext` into a deterministic preamble
/// prepended to the user prompt before the runner is spawned. Kept
/// as a free function so the prompt-shaping rules are covered by unit
/// tests independently of the bus / registry plumbing.
///
/// The preamble is only emitted when at least one context field is
/// populated; otherwise the prompt passes through unchanged so
/// short-prompt fidelity (e.g. "what time is it") is preserved.
pub(super) fn build_chat_prompt(
    ctx: Option<&codeless_rpc::ChatContext>,
    job_spec_block: Option<&str>,
    job_refs_block: Option<&str>,
    mode: ChatMode,
    prompt: &str,
) -> String {
    let ctx_has_any = ctx.is_some_and(|c| {
        c.ui_location.is_some()
            || c.selection.is_some()
            || !c.attachments.is_empty()
            || !c.user_prompts.is_empty()
    });
    let job_has_any = job_spec_block.is_some_and(|s| !s.is_empty());
    let refs_has_any = job_refs_block.is_some_and(|s| !s.is_empty());
    let spec_mode = mode == ChatMode::Spec;
    if !ctx_has_any && !job_has_any && !refs_has_any && !spec_mode {
        return prompt.to_owned();
    }

    let mut out = String::new();
    if spec_mode {
        out.push_str(SPEC_MODE_BANNER);
        out.push('\n');
    }
    out.push_str("# Context\n\n");
    if let Some(block) = job_spec_block.filter(|s| !s.is_empty()) {
        out.push_str(block);
        if !block.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    if let Some(block) = job_refs_block.filter(|s| !s.is_empty()) {
        out.push_str(block);
        if !block.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    let Some(ctx) = ctx else {
        out.push_str("# Request\n\n");
        out.push_str(prompt);
        return out;
    };
    if let Some(loc) = ctx.ui_location.as_deref() {
        out.push_str(&format!("User is viewing: {loc}\n\n"));
    }
    if !ctx.attachments.is_empty() {
        out.push_str("Files attached (paths are relative to the working directory):\n");
        for a in &ctx.attachments {
            match a.mime_type.as_deref() {
                Some(mt) => out.push_str(&format!("- {} ({mt})\n", a.relative_path)),
                None => out.push_str(&format!("- {}\n", a.relative_path)),
            }
        }
        out.push('\n');
    }
    if let Some(sel) = ctx.selection.as_deref() {
        out.push_str("Current selection:\n```\n");
        out.push_str(sel);
        if !sel.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");
    }
    for snippet in &ctx.user_prompts {
        out.push_str(&format!("## {}\n\n{}\n\n", snippet.label, snippet.body));
    }
    out.push_str("# Request\n\n");
    out.push_str(prompt);
    out
}

fn truncate_for_chat(s: &str) -> std::borrow::Cow<'_, str> {
    truncate_with_cap(s, MAX_CHAT_SPEC_BYTES)
}

fn truncate_with_cap(s: &str, cap: usize) -> std::borrow::Cow<'_, str> {
    if s.len() <= cap {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut cut = cap;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 64);
    out.push_str(&s[..cut]);
    out.push_str("\n\n[…truncated for chat preamble; read the full file from disk if needed…]\n");
    std::borrow::Cow::Owned(out)
}

/// Top-of-prompt banner the spec-mode preamble opens with. Mirrors the
/// claude-code plan-mode model: the user has explicitly flipped the chat
/// into "I am here to shape the job spec, not to run it," and the agent
/// must respect that even when the conversation drifts toward
/// implementation details.
const SPEC_MODE_BANNER: &str = "# Spec mode (active)\n\n\
The user has flipped this chat into SPEC MODE. You are authoring the \
job's spec, not implementing it. Edit only files under \
`.codeless/jobs/<name>/` (template.yaml, SCOPE.md, WORKFLOW.md, and \
per-stage `*.md`). Do NOT edit repo source code, run shell commands, \
commit, push, or invoke the network. Your tool surface has been \
restricted to read + edit + write + grep on the worktree; calls to \
disallowed tools will fail.\n\n\
If the user asks you to implement something rather than describe it, \
remind them they are in spec mode and either (a) capture the request \
as a stage in `template.yaml` for them to run later, or (b) suggest \
they flip back to work mode.\n";

/// Tool list passed to the CLI wrapper via `CliCfg::allowed_tools`
/// when the chat turn is spec-mode. Keep in sync with the banner above —
/// if a tool is mentioned there as "available" it must be in this list.
const SPEC_MODE_ALLOWED_TOOLS: &str = "Read,Edit,Write,Glob,Grep,LS,TodoWrite";

/// Tells the chat agent it owns the job's spec files and how to edit
/// them safely. Appended after the spec fold so the agent has the
/// current contents in mind before it reads the rules.
///
/// Disk is the source of truth at run-time: `start_job` / `resume_job`
/// re-parse `template.yaml` from disk and refresh the DB row before
/// transitioning to Queued. The agent must not touch `CHAT.md` — the
/// runtime appends to it on every turn.
const CHAT_JOB_SPEC_AUTHORING_PRIMER: &str = "## Job-spec authoring\n\n\
You may edit this job's spec directly using your ambient `Edit`, `Write`, \
and `Read` tools on files under `.codeless/jobs/<name>/`:\n\n\
- `template.yaml` — name, goal, `stages[]`. The `name:` field is \
immutable; changing it will cause the next `start_job` to fail. Other \
edits land on the next run.\n\
- `SCOPE.md` — load-bearing scope, folded into every stage prompt.\n\
- `WORKFLOW.md` — per-stage protocol, end-of-stage gate, drift rules.\n\
- Per-stage `*.md` — referenced from `stages[i].docs:` and folded into \
that stage's prompt only.\n\n\
Do NOT touch `CHAT.md`; the runtime appends to it on every turn.\n\n\
When the user clicks **run**, the runtime re-parses `template.yaml` \
from disk into SQLite, so your edits take effect without any explicit \
save. A malformed `template.yaml` will surface as an `InvalidArgument` \
on `start_job` — keep YAML valid before handing back.\n";

/// Per-file byte budget when folding job spec files into the chat
/// preamble. Sized to leave room for two large files plus the user's
/// transcript without crowding the model's input window.
const MAX_CHAT_SPEC_BYTES: usize = 8 * 1024;

/// Independent byte cap for one referenced job's recent-activity fold.
/// Sized smaller than the spec cap because a `job_report` rendering is
/// dense (stage list + tool-call counts + per-turn rows) and several
/// refs can stack on a single turn — a smaller per-ref budget keeps
/// the combined section bounded without coordinating across refs.
const MAX_CHAT_HISTORY_BYTES: usize = 4 * 1024;

/// Default cap on how many recent turns one referenced job contributes
/// to the preamble when the caller does not override it. Five turns is
/// usually enough to characterise "what is this job up to" without
/// pulling in the entire conversation.
const DEFAULT_REF_HISTORY_TURN_LIMIT: u32 = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::{AddRepoArgs, ChatContext, JobContextRef, RpcServer, SubmitJobArgs};
    use codeless_types::{GitAuth, JobId};
    use std::process::Command;
    use tempfile::TempDir;

    const TEMPLATE_A: &str = "name: alpha\ngoal: g\nstages:\n  - one\n";
    const TEMPLATE_B: &str = "name: bravo\ngoal: g\nstages:\n  - one\n";

    fn init_git_repo(dir: &std::path::Path) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "test"][..],
            &["commit", "--allow-empty", "-q", "-m", "root"][..],
        ] {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args.iter().copied())
                .output()
                .expect("spawn git");
            assert!(out.status.success(), "git {args:?}: {:?}", out);
        }
    }

    async fn submit_seed(
        rpc: &InProcessRpc,
        repo_id: codeless_types::RepoId,
        template_yaml: &str,
        branch: &str,
    ) -> JobId {
        let job = rpc
            .submit_job(SubmitJobArgs {
                repo_id,
                prompt: None,
                template_yaml: Some(template_yaml.into()),
                runner: "mock".into(),
                branch: branch.into(),
                workspace_mode: Some(codeless_types::WorkspaceMode::Worktree),
                cost_cap_cents: 0,
                wall_clock_cap_ms: 0,
                model: None,
                permission_mode: None,
                effort: None,
                system_prompt: None,
                start_immediately: true,
            })
            .await
            .unwrap();
        job.id
    }

    async fn add_repo(
        rpc: &InProcessRpc,
        name: &str,
        dir: &std::path::Path,
    ) -> codeless_types::Repo {
        rpc.add_repo(AddRepoArgs {
            name: name.into(),
            clone_url: format!("https://example.test/{name}.git"),
            default_branch: "main".into(),
            local_path: dir.to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap()
    }

    #[test]
    fn build_chat_prompt_passes_through_when_nothing_set() {
        let out = build_chat_prompt(None, None, None, ChatMode::Work, "hello");
        assert_eq!(out, "hello");
    }

    #[test]
    fn build_chat_prompt_includes_refs_block_after_spec() {
        let ctx = ChatContext::default();
        let out = build_chat_prompt(
            Some(&ctx),
            Some("ACTIVE SPEC"),
            Some("REFS BLOCK"),
            ChatMode::Work,
            "do thing",
        );
        let active_at = out.find("ACTIVE SPEC").expect("active spec rendered");
        let refs_at = out.find("REFS BLOCK").expect("refs rendered");
        let req_at = out.find("# Request").expect("request header rendered");
        assert!(active_at < refs_at, "refs should follow active spec");
        assert!(refs_at < req_at, "refs should precede the user request");
        assert!(out.ends_with("do thing"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_empty_returns_none() {
        let rpc = InProcessRpc::new().await.unwrap();
        let out = load_chat_job_refs(&rpc, None, &[]).await.unwrap();
        assert!(out.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_without_active_repo_is_rejected() {
        let rpc = InProcessRpc::new().await.unwrap();
        let err = load_chat_job_refs(
            &rpc,
            None,
            &[JobContextRef {
                job_id: JobId::new(),
                include_spec: true,
                include_history: false,
                history_turn_limit: None,
            }],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_cross_repo_rejected() {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        init_git_repo(tmp_a.path());
        init_git_repo(tmp_b.path());

        let rpc = InProcessRpc::new().await.unwrap();
        let repo_a = add_repo(&rpc, "alpha", tmp_a.path()).await;
        let repo_b = add_repo(&rpc, "bravo", tmp_b.path()).await;
        let _job_a = submit_seed(&rpc, repo_a.id, TEMPLATE_A, "codeless/job-a").await;
        let job_b = submit_seed(&rpc, repo_b.id, TEMPLATE_B, "codeless/job-b").await;

        let err = load_chat_job_refs(
            &rpc,
            Some(repo_a.id),
            &[JobContextRef {
                job_id: job_b,
                include_spec: true,
                include_history: false,
                history_turn_limit: None,
            }],
        )
        .await
        .unwrap_err();
        match err {
            RpcError::InvalidArgument(m) => assert!(m.contains("different repo"), "msg: {m}"),
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_spec_only_renders_section() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = add_repo(&rpc, "demo", tmp.path()).await;
        let job_a = submit_seed(&rpc, repo.id, TEMPLATE_A, "codeless/job-a").await;
        let _job_b = submit_seed(&rpc, repo.id, TEMPLATE_B, "codeless/job-b").await;

        let out = load_chat_job_refs(
            &rpc,
            Some(repo.id),
            &[JobContextRef {
                job_id: job_a,
                include_spec: true,
                include_history: false,
                history_turn_limit: None,
            }],
        )
        .await
        .unwrap()
        .expect("ref block present");
        assert!(out.contains("## Referenced jobs"));
        assert!(out.contains("alpha"));
        assert!(out.contains("template.yaml"));
        assert!(!out.contains("Recent activity"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_history_only_renders_recent_activity_header() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = add_repo(&rpc, "demo", tmp.path()).await;
        let job_a = submit_seed(&rpc, repo.id, TEMPLATE_A, "codeless/job-a").await;

        let out = load_chat_job_refs(
            &rpc,
            Some(repo.id),
            &[JobContextRef {
                job_id: job_a,
                include_spec: false,
                include_history: true,
                history_turn_limit: Some(5),
            }],
        )
        .await
        .unwrap()
        .expect("ref block present");
        assert!(out.contains("Recent activity"));
        assert!(!out.contains("template.yaml"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_spec_and_history_render_both() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = add_repo(&rpc, "demo", tmp.path()).await;
        let job_a = submit_seed(&rpc, repo.id, TEMPLATE_A, "codeless/job-a").await;

        let out = load_chat_job_refs(
            &rpc,
            Some(repo.id),
            &[JobContextRef {
                job_id: job_a,
                include_spec: true,
                include_history: true,
                history_turn_limit: None,
            }],
        )
        .await
        .unwrap()
        .expect("ref block present");
        assert!(out.contains("template.yaml"));
        assert!(out.contains("Recent activity"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_refs_truncates_oversized_spec_file() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let rpc = InProcessRpc::new().await.unwrap();
        let repo = add_repo(&rpc, "demo", tmp.path()).await;
        let job_a = submit_seed(&rpc, repo.id, TEMPLATE_A, "codeless/job-a").await;

        let scope_path = tmp.path().join(".codeless/jobs/alpha/SCOPE.md");
        let mut huge = String::with_capacity(MAX_CHAT_SPEC_BYTES * 2);
        while huge.len() < MAX_CHAT_SPEC_BYTES * 2 {
            huge.push_str("padding line that is longer than zero bytes\n");
        }
        std::fs::write(&scope_path, &huge).unwrap();

        let out = load_chat_job_refs(
            &rpc,
            Some(repo.id),
            &[JobContextRef {
                job_id: job_a,
                include_spec: true,
                include_history: false,
                history_turn_limit: None,
            }],
        )
        .await
        .unwrap()
        .expect("ref block present");
        assert!(out.contains("truncated for chat preamble"));
    }
}
