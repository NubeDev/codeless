use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use codeless_rpc::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, ApproveScopePatchArgs, AttachWorkspaceArgs,
    AttachWorkspaceResult, CancelAssistantActionArgs, CancelAssistantActionResult,
    CancelChatTaskArgs, CommentReviewArgs, ConfirmAssistantActionArgs,
    ConfirmAssistantActionResult, CreateAssistantThreadArgs, DeleteAssistantThreadArgs,
    DeleteJobArgs, DeleteJobFileArgs, DeletePersonaArgs, DetachWorkspaceArgs,
    DraftJobFromConversationArgs, EditScopePatchArgs, FsCreateDirArgs, FsCreateFileArgs,
    FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs, FsReadDirResult, FsReadFileArgs,
    FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, GetPersonaArgs, JobDiffArgs, JobDiffResult, JobReportArgs,
    JobReportResult, ListAssistantMessagesArgs, ListAssistantMessagesResult,
    ListAssistantThreadsArgs, ListAssistantThreadsResult, ListJobFilesArgs, ListJobFilesResult,
    ListJobsArgs, ListJobsResult, ListPersonasArgs, ListPersonasResult, ListProposedPatchesArgs,
    ListProposedPatchesResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    ListScheduledPausePointsArgs, ListScheduledPausePointsResult, ListStagesArgs, ListStagesResult,
    ListWorkspacesResult, OverridePreCheckAndResumeArgs, PauseJobArgs, ReadJobFileArgs,
    ReadJobFileResult, RejectScopePatchArgs, RemoveRepoArgs, RerunJobArgs, ResetJobArgs,
    ResumeJobArgs, RevertScopePatchArgs, RevertScopePatchResult, RpcError, ScopePatchActionResult,
    ServerInfo, SetJobPolicyArgs, StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs,
    StopReviewArgs, SubmitJobArgs, UpdateJobArgs, UpdateJobScopeArgs, UpdateJobScopeResult,
    UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadAssistantAttachmentArgs,
    UploadAssistantAttachmentResult, UploadChatAttachmentArgs, UploadChatAttachmentResult,
    UpsertPersonaArgs, ValidateWorkspacePathArgs, ValidateWorkspacePathResult, WriteHandoverArgs,
    WriteHandoverResult, WriteJobFileArgs, WriteJobFileResult,
};
use codeless_types::{AssistantThread, Job, Persona, Repo, Review};
use serde_json::Value;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::{auth::bearer_layer, sse::events_handler, AppState};

pub(crate) fn router(state: AppState) -> Router {
    let rpc_routes = Router::new()
        .route("/rpc/add_repo", post(add_repo))
        .route("/rpc/remove_repo", post(remove_repo))
        .route("/rpc/list_repos", post(list_repos))
        .route("/rpc/submit_job", post(submit_job))
        .route("/rpc/get_job", post(get_job))
        .route("/rpc/list_jobs", post(list_jobs))
        .route("/rpc/list_stages", post(list_stages))
        .route(
            "/rpc/list_scheduled_pause_points",
            post(list_scheduled_pause_points),
        )
        .route("/rpc/job_report", post(job_report))
        .route("/rpc/stop_job", post(stop_job))
        .route("/rpc/pause_job", post(pause_job))
        .route("/rpc/start_job", post(start_job))
        .route("/rpc/resume_job", post(resume_job))
        .route(
            "/rpc/override_pre_check_and_resume",
            post(override_pre_check_and_resume),
        )
        .route("/rpc/rerun_job", post(rerun_job))
        .route("/rpc/reset_job", post(reset_job))
        .route("/rpc/update_job", post(update_job))
        .route("/rpc/delete_job", post(delete_job))
        .route("/rpc/gc_worktrees", post(gc_worktrees))
        .route("/rpc/job_diff", post(job_diff))
        .route("/rpc/list_reviews", post(list_reviews))
        .route("/rpc/approve_review", post(approve_review))
        .route("/rpc/comment_review", post(comment_review))
        .route("/rpc/stop_review", post(stop_review))
        .route("/rpc/fs_read_dir", post(fs_read_dir))
        .route("/rpc/fs_read_file", post(fs_read_file))
        .route("/rpc/fs_write_file", post(fs_write_file))
        .route("/rpc/fs_stat", post(fs_stat))
        .route("/rpc/fs_cwd", post(fs_cwd))
        .route("/rpc/fs_create_file", post(fs_create_file))
        .route("/rpc/fs_create_dir", post(fs_create_dir))
        .route("/rpc/fs_move", post(fs_move))
        .route("/rpc/fs_delete", post(fs_delete))
        .route("/rpc/list_job_files", post(list_job_files))
        .route("/rpc/read_job_file", post(read_job_file))
        .route("/rpc/write_job_file", post(write_job_file))
        .route("/rpc/delete_job_file", post(delete_job_file))
        .route("/rpc/update_job_template", post(update_job_template))
        .route("/rpc/write_handover", post(write_handover))
        .route("/rpc/agent_chat", post(agent_chat))
        .route("/rpc/upload_chat_attachment", post(upload_chat_attachment))
        .route("/rpc/cancel_chat_task", post(cancel_chat_task))
        .route("/rpc/stop_active", post(stop_active))
        .route("/rpc/attach_workspace", post(attach_workspace))
        .route("/rpc/detach_workspace", post(detach_workspace))
        .route("/rpc/list_workspaces", post(list_workspaces))
        .route(
            "/rpc/validate_workspace_path",
            post(validate_workspace_path),
        )
        .route("/rpc/list_assistant_threads", post(list_assistant_threads))
        .route(
            "/rpc/create_assistant_thread",
            post(create_assistant_thread),
        )
        .route(
            "/rpc/delete_assistant_thread",
            post(delete_assistant_thread),
        )
        .route(
            "/rpc/upload_assistant_attachment",
            post(upload_assistant_attachment),
        )
        .route(
            "/rpc/list_assistant_messages",
            post(list_assistant_messages),
        )
        .route(
            "/rpc/append_assistant_message",
            post(append_assistant_message),
        )
        .route(
            "/rpc/confirm_assistant_action",
            post(confirm_assistant_action),
        )
        .route(
            "/rpc/cancel_assistant_action",
            post(cancel_assistant_action),
        )
        .route("/rpc/update_job_scope", post(update_job_scope))
        .route(
            "/rpc/draft_job_from_conversation",
            post(draft_job_from_conversation),
        )
        .route("/rpc/list_personas", post(list_personas))
        .route("/rpc/get_persona", post(get_persona))
        .route("/rpc/upsert_persona", post(upsert_persona))
        .route("/rpc/delete_persona", post(delete_persona))
        .route("/rpc/approve_scope_patch", post(approve_scope_patch))
        .route("/rpc/reject_scope_patch", post(reject_scope_patch))
        .route("/rpc/edit_scope_patch", post(edit_scope_patch))
        .route("/rpc/revert_scope_patch", post(revert_scope_patch))
        .route("/rpc/list_proposed_patches", post(list_proposed_patches))
        .route("/rpc/set_job_policy", post(set_job_policy));
    // `GET /plugins` rides alongside the `/rpc/*` routes inside the
    // bearer-gated sub-router: discovery of which plugins are loaded
    // is operator-facing and must not leak before a token is supplied.
    // The ServeDir mounts at `/plugins/<id>/ui/*` are intentionally
    // outside this gate -- see `plugins::ui_routes`.
    let rpc_routes = if let Some(r) = crate::plugins::bearer_routes(&state) {
        rpc_routes.merge(r)
    } else {
        rpc_routes
    }
    .layer(middleware::from_fn_with_state(state.clone(), bearer_layer));

    let events = Router::new().route("/events", get(events_handler));

    // `/healthz` and `/version` sit outside the bearer gate so probes
    // and human curl-the-server checks work without provisioning a
    // token. Neither leaks state: healthz returns a constant, version
    // returns the crate version baked in at build time.
    let unauthenticated = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/version",
            get(|| async { env!("CARGO_PKG_VERSION").to_string() }),
        )
        .route("/server/info", get(server_info));

    // Permissive CORS is correct for the single-tenant MVP: the
    // server binds loopback by default (R5), so "any origin" only
    // covers other processes on the same host that already have
    // local-file access — i.e. no real reduction in trust boundary.
    // Phase 7's OIDC story will replace this with an explicit
    // allowlist alongside cookie-based auth.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Request-tracing layer. The defaults emit a span per request
    // (method + uri + status + latency); operator-facing logs land
    // on stderr through whatever `tracing-subscriber` the binary
    // initialised. Subscribers that don't filter `tower_http` in
    // see one event per request at info level.
    let trace = TraceLayer::new_for_http();

    // ai-ui surface (`/api/ai-ui/*`). Mounted only when the consumer
    // built an `AiUiState`; merged outside the bearer middleware so the
    // OpenUI frontend can call it without learning codeless's token —
    // R5's loopback-bind default is what keeps single-tenant deployments
    // safe. See `src/ai_ui.rs` for the per-route detail.
    let ai_ui_router = state.ai_ui.is_some().then(crate::ai_ui::router);

    // Per-plugin `ServeDir` mounts for `/plugins/<id>/ui/*`. Mounted
    // outside the bearer gate: the bundle bytes are no more sensitive
    // than the host UI bundle the browser already loads (the bearer
    // protects the *listing* of which plugins exist, not the bundles
    // themselves). See `DOCS/plugins/PLUGIN-UI-FEDERATION.md` §
    // Server wiring.
    let plugin_ui = crate::plugins::ui_routes(&state);

    let mut app = Router::new()
        .merge(rpc_routes)
        .merge(events)
        .merge(unauthenticated);
    if let Some(r) = ai_ui_router {
        app = app.merge(r);
    }
    if let Some(r) = plugin_ui {
        app = app.merge(r);
    }
    app.layer(cors).layer(trace).with_state(state)
}

/// Map a typed `RpcError` to the HTTP response shape the browser
/// client decodes in `RpcError.fromHttpStatus`. The mapping is
/// wire-stable: renaming a variant on the Rust side or changing the
/// status here is a breaking change for the UI.
///
/// `Workspace(WorkspaceError)` rides on the existing 409 channel but
/// carries a JSON body the client parses into the typed variant — the
/// UI then branches on `WorkspaceError` directly rather than string-
/// matching on a `Conflict` message.
fn map_err(err: RpcError) -> (StatusCode, String) {
    match err {
        RpcError::NotFound(m) => (StatusCode::NOT_FOUND, m),
        RpcError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m),
        RpcError::Conflict(m) => (StatusCode::CONFLICT, m),
        RpcError::Workspace(payload) => (
            StatusCode::CONFLICT,
            serde_json::to_string(&payload)
                .expect("WorkspaceError always serialises (derive Serialize)"),
        ),
        RpcError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}

type HandlerResult<T> = Result<Json<T>, (StatusCode, String)>;

async fn add_repo(
    State(st): State<AppState>,
    Json(args): Json<AddRepoArgs>,
) -> HandlerResult<Repo> {
    st.rpc.add_repo(args).await.map(Json).map_err(map_err)
}

async fn remove_repo(
    State(st): State<AppState>,
    Json(args): Json<RemoveRepoArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .remove_repo(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

/// `list_repos` takes no args but the browser still sends `{}` (the
/// TS type is `Record<string, never>`). Accept the body as
/// `Option<Json<Value>>` so missing, empty, or `{}` payloads all
/// succeed — anything richer is silently ignored, which matches the
/// trait method's actual signature.
async fn list_repos(
    State(st): State<AppState>,
    _body: Option<Json<Value>>,
) -> HandlerResult<ListReposResult> {
    st.rpc.list_repos().await.map(Json).map_err(map_err)
}

async fn submit_job(
    State(st): State<AppState>,
    Json(args): Json<SubmitJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.submit_job(args).await.map(Json).map_err(map_err)
}

async fn get_job(State(st): State<AppState>, Json(args): Json<GetJobArgs>) -> HandlerResult<Job> {
    st.rpc.get_job(args).await.map(Json).map_err(map_err)
}

async fn list_jobs(
    State(st): State<AppState>,
    Json(args): Json<ListJobsArgs>,
) -> HandlerResult<ListJobsResult> {
    st.rpc.list_jobs(args).await.map(Json).map_err(map_err)
}

async fn list_stages(
    State(st): State<AppState>,
    Json(args): Json<ListStagesArgs>,
) -> HandlerResult<ListStagesResult> {
    st.rpc.list_stages(args).await.map(Json).map_err(map_err)
}

async fn list_scheduled_pause_points(
    State(st): State<AppState>,
    Json(args): Json<ListScheduledPausePointsArgs>,
) -> HandlerResult<ListScheduledPausePointsResult> {
    st.rpc
        .list_scheduled_pause_points(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn job_report(
    State(st): State<AppState>,
    Json(args): Json<JobReportArgs>,
) -> HandlerResult<JobReportResult> {
    st.rpc.job_report(args).await.map(Json).map_err(map_err)
}

async fn stop_job(
    State(st): State<AppState>,
    Json(args): Json<StopJobArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .stop_job(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn pause_job(
    State(st): State<AppState>,
    Json(args): Json<PauseJobArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .pause_job(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn start_job(
    State(st): State<AppState>,
    Json(args): Json<StartJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.start_job(args).await.map(Json).map_err(map_err)
}

async fn rerun_job(
    State(st): State<AppState>,
    Json(args): Json<RerunJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.rerun_job(args).await.map(Json).map_err(map_err)
}

async fn reset_job(
    State(st): State<AppState>,
    Json(args): Json<ResetJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.reset_job(args).await.map(Json).map_err(map_err)
}

async fn update_job(
    State(st): State<AppState>,
    Json(args): Json<UpdateJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.update_job(args).await.map(Json).map_err(map_err)
}

async fn delete_job(
    State(st): State<AppState>,
    Json(args): Json<DeleteJobArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .delete_job(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn resume_job(
    State(st): State<AppState>,
    Json(args): Json<ResumeJobArgs>,
) -> HandlerResult<Job> {
    st.rpc.resume_job(args).await.map(Json).map_err(map_err)
}

async fn override_pre_check_and_resume(
    State(st): State<AppState>,
    Json(args): Json<OverridePreCheckAndResumeArgs>,
) -> HandlerResult<Job> {
    st.rpc
        .override_pre_check_and_resume(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn gc_worktrees(
    State(st): State<AppState>,
    Json(args): Json<GcWorktreesArgs>,
) -> HandlerResult<GcWorktreesResult> {
    st.rpc.gc_worktrees(args).await.map(Json).map_err(map_err)
}

async fn job_diff(
    State(st): State<AppState>,
    Json(args): Json<JobDiffArgs>,
) -> HandlerResult<JobDiffResult> {
    st.rpc.job_diff(args).await.map(Json).map_err(map_err)
}

async fn list_reviews(
    State(st): State<AppState>,
    Json(args): Json<ListReviewsArgs>,
) -> HandlerResult<ListReviewsResult> {
    st.rpc.list_reviews(args).await.map(Json).map_err(map_err)
}

async fn approve_review(
    State(st): State<AppState>,
    Json(args): Json<ApproveReviewArgs>,
) -> HandlerResult<Review> {
    st.rpc.approve_review(args).await.map(Json).map_err(map_err)
}

async fn comment_review(
    State(st): State<AppState>,
    Json(args): Json<CommentReviewArgs>,
) -> HandlerResult<Review> {
    st.rpc.comment_review(args).await.map(Json).map_err(map_err)
}

async fn stop_review(
    State(st): State<AppState>,
    Json(args): Json<StopReviewArgs>,
) -> HandlerResult<Review> {
    st.rpc.stop_review(args).await.map(Json).map_err(map_err)
}

async fn fs_read_dir(
    State(st): State<AppState>,
    Json(args): Json<FsReadDirArgs>,
) -> HandlerResult<FsReadDirResult> {
    st.rpc.fs_read_dir(args).await.map(Json).map_err(map_err)
}

async fn fs_read_file(
    State(st): State<AppState>,
    Json(args): Json<FsReadFileArgs>,
) -> HandlerResult<FsReadFileResult> {
    st.rpc.fs_read_file(args).await.map(Json).map_err(map_err)
}

async fn fs_write_file(
    State(st): State<AppState>,
    Json(args): Json<FsWriteFileArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .fs_write_file(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn fs_stat(
    State(st): State<AppState>,
    Json(args): Json<FsStatArgs>,
) -> HandlerResult<FsStatResult> {
    st.rpc.fs_stat(args).await.map(Json).map_err(map_err)
}

async fn fs_cwd(
    State(st): State<AppState>,
    _body: Option<Json<Value>>,
) -> HandlerResult<FsCwdResult> {
    st.rpc.fs_cwd().await.map(Json).map_err(map_err)
}

async fn fs_create_file(
    State(st): State<AppState>,
    Json(args): Json<FsCreateFileArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .fs_create_file(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn fs_create_dir(
    State(st): State<AppState>,
    Json(args): Json<FsCreateDirArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .fs_create_dir(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn fs_move(State(st): State<AppState>, Json(args): Json<FsMoveArgs>) -> HandlerResult<Value> {
    st.rpc
        .fs_move(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn fs_delete(
    State(st): State<AppState>,
    Json(args): Json<FsDeleteArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .fs_delete(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

/// Unauthenticated snapshot of the server's configuration. The UI hits
/// this before it has a bearer token, so it must sit outside the
/// `/rpc/*` gate. Returning `ServerInfo` by clone keeps the response
/// owned and decoupled from the shared `Arc` inside `AppState`.
async fn server_info(State(st): State<AppState>) -> Json<ServerInfo> {
    Json((*st.server_info).clone())
}

async fn list_job_files(
    State(st): State<AppState>,
    Json(args): Json<ListJobFilesArgs>,
) -> HandlerResult<ListJobFilesResult> {
    st.rpc.list_job_files(args).await.map(Json).map_err(map_err)
}

async fn read_job_file(
    State(st): State<AppState>,
    Json(args): Json<ReadJobFileArgs>,
) -> HandlerResult<ReadJobFileResult> {
    st.rpc.read_job_file(args).await.map(Json).map_err(map_err)
}

async fn write_job_file(
    State(st): State<AppState>,
    Json(args): Json<WriteJobFileArgs>,
) -> HandlerResult<WriteJobFileResult> {
    st.rpc.write_job_file(args).await.map(Json).map_err(map_err)
}

async fn delete_job_file(
    State(st): State<AppState>,
    Json(args): Json<DeleteJobFileArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .delete_job_file(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn update_job_template(
    State(st): State<AppState>,
    Json(args): Json<UpdateJobTemplateArgs>,
) -> HandlerResult<UpdateJobTemplateResult> {
    st.rpc
        .update_job_template(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn write_handover(
    State(st): State<AppState>,
    Json(args): Json<WriteHandoverArgs>,
) -> HandlerResult<WriteHandoverResult> {
    st.rpc.write_handover(args).await.map(Json).map_err(map_err)
}

async fn agent_chat(
    State(st): State<AppState>,
    Json(args): Json<AgentChatArgs>,
) -> HandlerResult<AgentChatResult> {
    st.rpc.agent_chat(args).await.map(Json).map_err(map_err)
}

async fn upload_chat_attachment(
    State(st): State<AppState>,
    Json(args): Json<UploadChatAttachmentArgs>,
) -> HandlerResult<UploadChatAttachmentResult> {
    st.rpc
        .upload_chat_attachment(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn cancel_chat_task(
    State(st): State<AppState>,
    Json(args): Json<CancelChatTaskArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .cancel_chat_task(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn stop_active(
    State(st): State<AppState>,
    Json(args): Json<StopActiveArgs>,
) -> HandlerResult<StopActiveResult> {
    st.rpc.stop_active(args).await.map(Json).map_err(map_err)
}

async fn attach_workspace(
    State(st): State<AppState>,
    Json(args): Json<AttachWorkspaceArgs>,
) -> HandlerResult<AttachWorkspaceResult> {
    st.rpc
        .attach_workspace(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn detach_workspace(
    State(st): State<AppState>,
    Json(args): Json<DetachWorkspaceArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .detach_workspace(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn list_workspaces(
    State(st): State<AppState>,
    _body: Option<Json<Value>>,
) -> HandlerResult<ListWorkspacesResult> {
    st.rpc.list_workspaces().await.map(Json).map_err(map_err)
}

async fn validate_workspace_path(
    State(st): State<AppState>,
    Json(args): Json<ValidateWorkspacePathArgs>,
) -> HandlerResult<ValidateWorkspacePathResult> {
    st.rpc
        .validate_workspace_path(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn list_assistant_threads(
    State(st): State<AppState>,
    Json(args): Json<ListAssistantThreadsArgs>,
) -> HandlerResult<ListAssistantThreadsResult> {
    st.rpc
        .list_assistant_threads(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn create_assistant_thread(
    State(st): State<AppState>,
    Json(args): Json<CreateAssistantThreadArgs>,
) -> HandlerResult<AssistantThread> {
    st.rpc
        .create_assistant_thread(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn delete_assistant_thread(
    State(st): State<AppState>,
    Json(args): Json<DeleteAssistantThreadArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .delete_assistant_thread(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn upload_assistant_attachment(
    State(st): State<AppState>,
    Json(args): Json<UploadAssistantAttachmentArgs>,
) -> HandlerResult<UploadAssistantAttachmentResult> {
    st.rpc
        .upload_assistant_attachment(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn list_assistant_messages(
    State(st): State<AppState>,
    Json(args): Json<ListAssistantMessagesArgs>,
) -> HandlerResult<ListAssistantMessagesResult> {
    st.rpc
        .list_assistant_messages(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn append_assistant_message(
    State(st): State<AppState>,
    Json(args): Json<AppendAssistantMessageArgs>,
) -> HandlerResult<AppendAssistantMessageResult> {
    st.rpc
        .append_assistant_message(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn confirm_assistant_action(
    State(st): State<AppState>,
    Json(args): Json<ConfirmAssistantActionArgs>,
) -> HandlerResult<ConfirmAssistantActionResult> {
    st.rpc
        .confirm_assistant_action(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn cancel_assistant_action(
    State(st): State<AppState>,
    Json(args): Json<CancelAssistantActionArgs>,
) -> HandlerResult<CancelAssistantActionResult> {
    st.rpc
        .cancel_assistant_action(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn update_job_scope(
    State(st): State<AppState>,
    Json(args): Json<UpdateJobScopeArgs>,
) -> HandlerResult<UpdateJobScopeResult> {
    st.rpc
        .update_job_scope(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn draft_job_from_conversation(
    State(st): State<AppState>,
    Json(args): Json<DraftJobFromConversationArgs>,
) -> HandlerResult<Job> {
    st.rpc
        .draft_job_from_conversation(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn list_personas(
    State(st): State<AppState>,
    Json(args): Json<ListPersonasArgs>,
) -> HandlerResult<ListPersonasResult> {
    st.rpc.list_personas(args).await.map(Json).map_err(map_err)
}

async fn get_persona(
    State(st): State<AppState>,
    Json(args): Json<GetPersonaArgs>,
) -> HandlerResult<Persona> {
    st.rpc.get_persona(args).await.map(Json).map_err(map_err)
}

async fn upsert_persona(
    State(st): State<AppState>,
    Json(args): Json<UpsertPersonaArgs>,
) -> HandlerResult<Persona> {
    st.rpc.upsert_persona(args).await.map(Json).map_err(map_err)
}

async fn delete_persona(
    State(st): State<AppState>,
    Json(args): Json<DeletePersonaArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .delete_persona(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}

async fn approve_scope_patch(
    State(st): State<AppState>,
    Json(args): Json<ApproveScopePatchArgs>,
) -> HandlerResult<ScopePatchActionResult> {
    st.rpc
        .approve_scope_patch(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn reject_scope_patch(
    State(st): State<AppState>,
    Json(args): Json<RejectScopePatchArgs>,
) -> HandlerResult<ScopePatchActionResult> {
    st.rpc
        .reject_scope_patch(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn edit_scope_patch(
    State(st): State<AppState>,
    Json(args): Json<EditScopePatchArgs>,
) -> HandlerResult<ScopePatchActionResult> {
    st.rpc
        .edit_scope_patch(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn revert_scope_patch(
    State(st): State<AppState>,
    Json(args): Json<RevertScopePatchArgs>,
) -> HandlerResult<RevertScopePatchResult> {
    st.rpc
        .revert_scope_patch(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn list_proposed_patches(
    State(st): State<AppState>,
    Json(args): Json<ListProposedPatchesArgs>,
) -> HandlerResult<ListProposedPatchesResult> {
    st.rpc
        .list_proposed_patches(args)
        .await
        .map(Json)
        .map_err(map_err)
}

async fn set_job_policy(
    State(st): State<AppState>,
    Json(args): Json<SetJobPolicyArgs>,
) -> HandlerResult<Value> {
    st.rpc
        .set_job_policy(args)
        .await
        .map(|()| Json(Value::Null))
        .map_err(map_err)
}
