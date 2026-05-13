use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use codeless_rpc::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, ApproveReviewArgs, CommentReviewArgs,
    DeleteJobFileArgs, FsCwdResult, FsReadDirArgs, FsReadDirResult, FsReadFileArgs,
    FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffResult, ListJobFilesArgs,
    ListJobFilesResult, ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs,
    ListReviewsResult, ListStagesArgs, ListStagesResult, ReadJobFileArgs, ReadJobFileResult,
    RemoveRepoArgs, RerunJobArgs, RpcError, ServerInfo, StartJobArgs, StopJobArgs, StopReviewArgs,
    SubmitJobArgs, UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadChatAttachmentArgs,
    UploadChatAttachmentResult, WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs,
    WriteJobFileResult,
};
use codeless_types::{Job, Repo, Review};
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
        .route("/rpc/stop_job", post(stop_job))
        .route("/rpc/start_job", post(start_job))
        .route("/rpc/rerun_job", post(rerun_job))
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
        .route("/rpc/list_job_files", post(list_job_files))
        .route("/rpc/read_job_file", post(read_job_file))
        .route("/rpc/write_job_file", post(write_job_file))
        .route("/rpc/delete_job_file", post(delete_job_file))
        .route("/rpc/update_job_template", post(update_job_template))
        .route("/rpc/write_handover", post(write_handover))
        .route("/rpc/agent_chat", post(agent_chat))
        .route(
            "/rpc/upload_chat_attachment",
            post(upload_chat_attachment),
        )
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

    Router::new()
        .merge(rpc_routes)
        .merge(events)
        .merge(unauthenticated)
        .layer(cors)
        .layer(trace)
        .with_state(state)
}

/// Map a typed `RpcError` to the HTTP response shape the browser
/// client decodes in `RpcError.fromHttpStatus`. The mapping is
/// wire-stable: renaming a variant on the Rust side or changing the
/// status here is a breaking change for the UI.
fn map_err(err: RpcError) -> (StatusCode, String) {
    match err {
        RpcError::NotFound(m) => (StatusCode::NOT_FOUND, m),
        RpcError::InvalidArgument(m) => (StatusCode::BAD_REQUEST, m),
        RpcError::Conflict(m) => (StatusCode::CONFLICT, m),
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
