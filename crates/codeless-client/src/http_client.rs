use async_trait::async_trait;
use codeless_rpc::{
    AddRepoArgs, ApproveReviewArgs, CommentReviewArgs, EventFilter, EventStream, FsCwdResult,
    FsReadDirArgs, FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult,
    FsWriteFileArgs, GcWorktreesArgs, GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffResult,
    ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult,
    RemoveRepoArgs, RerunJobArgs, RpcError, RpcResult, RpcServer, Since, StopJobArgs,
    StopReviewArgs, SubmitJobArgs,
};
use codeless_types::{Job, Repo, Review};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::Serialize;
use thiserror::Error;

use crate::sse::SseParser;

/// Caller-supplied configuration. The `base_url` is the server origin
/// without a trailing slash (`https://core.example.com`, not `…/`);
/// callers are responsible for trimming whitespace and stripping the
/// slash so the per-method `format!` here stays a one-liner.
#[derive(Debug, Clone)]
pub struct HttpRpcClientConfig {
    pub base_url: String,
    pub token: Option<String>,
}

/// Errors that can surface before the wire even returns — DNS,
/// connect, TLS, body-read. `RpcError` covers the *protocol* side
/// (the server answered with a typed status); `ClientError` covers
/// transport layer failures and is converted into
/// `RpcError::Internal` when the trait demands it.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("invalid base url: {0}")]
    InvalidUrl(String),
}

/// Implements `RpcServer` over `codeless-server`'s REST + SSE wire.
/// Constructed once per process and shared (`Arc<dyn RpcServer>`)
/// across call sites; the inner `reqwest::Client` pools connections.
pub struct HttpRpcClient {
    inner: Client,
    cfg: HttpRpcClientConfig,
}

impl HttpRpcClient {
    pub fn new(cfg: HttpRpcClientConfig) -> Result<Self, ClientError> {
        if cfg.base_url.is_empty() {
            return Err(ClientError::InvalidUrl("base_url is empty".into()));
        }
        if cfg.base_url.ends_with('/') {
            return Err(ClientError::InvalidUrl(
                "base_url must not end with a trailing slash".into(),
            ));
        }
        let inner = Client::builder().build().map_err(ClientError::Transport)?;
        Ok(Self { inner, cfg })
    }

    async fn call<A, R>(&self, method: &str, args: &A) -> RpcResult<R>
    where
        A: Serialize + ?Sized,
        R: serde::de::DeserializeOwned,
    {
        let url = format!("{}/rpc/{method}", self.cfg.base_url);
        let mut req = self.inner.post(&url).json(args);
        if let Some(t) = &self.cfg.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(transport_to_rpc)?;
        let status = resp.status();
        if status.is_success() {
            return resp.json::<R>().await.map_err(transport_to_rpc);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(status_to_rpc(status, body))
    }

    /// Same as `call` but discards the body — used by the void-result
    /// RPC methods (`remove_repo`, `stop_job`). The server still emits
    /// `null` JSON, but we don't bind it to a type.
    async fn call_void<A>(&self, method: &str, args: &A) -> RpcResult<()>
    where
        A: Serialize + ?Sized,
    {
        let url = format!("{}/rpc/{method}", self.cfg.base_url);
        let mut req = self.inner.post(&url).json(args);
        if let Some(t) = &self.cfg.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(transport_to_rpc)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(status_to_rpc(status, body))
    }
}

fn transport_to_rpc(err: reqwest::Error) -> RpcError {
    RpcError::Internal(format!("transport: {err}"))
}

/// Inverse of `codeless-server::routes::map_err`. Keep the match arms
/// in lock-step with the server side: drift means an `RpcError`
/// variant on one side surfaces as a different one on the other.
fn status_to_rpc(status: StatusCode, body: String) -> RpcError {
    let msg = if body.is_empty() {
        status.canonical_reason().unwrap_or("unknown").to_string()
    } else {
        body
    };
    match status {
        StatusCode::NOT_FOUND => RpcError::NotFound(msg),
        StatusCode::BAD_REQUEST => RpcError::InvalidArgument(msg),
        StatusCode::CONFLICT => RpcError::Conflict(msg),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            RpcError::Internal(format!("unauthorized ({}): {msg}", status.as_u16()))
        }
        _ => RpcError::Internal(format!("http {}: {msg}", status.as_u16())),
    }
}

/// `list_repos` carries no args but the wire still requires a JSON
/// body — both `HttpSseClient` and the axum router accept `{}`. A
/// unit struct serialises as `null`, which the server's
/// `Option<Json<Value>>` extractor would accept but the browser
/// never sends. Keeping the empty object shape preserves parity.
#[derive(Serialize)]
struct EmptyArgs {}

#[async_trait]
impl RpcServer for HttpRpcClient {
    async fn add_repo(&self, args: AddRepoArgs) -> RpcResult<Repo> {
        self.call("add_repo", &args).await
    }

    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()> {
        self.call_void("remove_repo", &args).await
    }

    async fn list_repos(&self) -> RpcResult<ListReposResult> {
        self.call("list_repos", &EmptyArgs {}).await
    }

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job> {
        self.call("submit_job", &args).await
    }

    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job> {
        self.call("get_job", &args).await
    }

    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
        self.call("list_jobs", &args).await
    }

    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()> {
        self.call_void("stop_job", &args).await
    }

    async fn rerun_job(&self, args: RerunJobArgs) -> RpcResult<Job> {
        self.call("rerun_job", &args).await
    }

    async fn gc_worktrees(&self, args: GcWorktreesArgs) -> RpcResult<GcWorktreesResult> {
        self.call("gc_worktrees", &args).await
    }

    async fn job_diff(&self, args: JobDiffArgs) -> RpcResult<JobDiffResult> {
        self.call("job_diff", &args).await
    }

    async fn list_reviews(&self, args: ListReviewsArgs) -> RpcResult<ListReviewsResult> {
        self.call("list_reviews", &args).await
    }

    async fn approve_review(&self, args: ApproveReviewArgs) -> RpcResult<Review> {
        self.call("approve_review", &args).await
    }

    async fn comment_review(&self, args: CommentReviewArgs) -> RpcResult<Review> {
        self.call("comment_review", &args).await
    }

    async fn stop_review(&self, args: StopReviewArgs) -> RpcResult<Review> {
        self.call("stop_review", &args).await
    }

    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream> {
        let url = build_subscribe_url(&self.cfg, &filter, since);
        let resp = self
            .inner
            .get(&url)
            .header("accept", "text/event-stream")
            .send()
            .await
            .map_err(transport_to_rpc)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(status_to_rpc(status, body));
        }

        let mut body = resp.bytes_stream();
        let stream = async_stream::stream! {
            let mut parser = SseParser::default();
            while let Some(chunk) = body.next().await {
                match chunk {
                    Ok(bytes) => {
                        for item in parser.feed(&bytes) {
                            yield item;
                        }
                    }
                    Err(err) => {
                        yield Err(transport_to_rpc(err));
                        // A transport failure ends the stream — the
                        // server-side cursor replay is the recovery
                        // path, driven by the caller re-subscribing
                        // with the last seen cursor.
                        return;
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }

    async fn fs_read_dir(&self, args: FsReadDirArgs) -> RpcResult<FsReadDirResult> {
        self.call("fs_read_dir", &args).await
    }

    async fn fs_read_file(&self, args: FsReadFileArgs) -> RpcResult<FsReadFileResult> {
        self.call("fs_read_file", &args).await
    }

    async fn fs_write_file(&self, args: FsWriteFileArgs) -> RpcResult<()> {
        self.call_void("fs_write_file", &args).await
    }

    async fn fs_stat(&self, args: FsStatArgs) -> RpcResult<FsStatResult> {
        self.call("fs_stat", &args).await
    }

    async fn fs_cwd(&self) -> RpcResult<FsCwdResult> {
        self.call("fs_cwd", &serde_json::json!({})).await
    }
}

fn build_subscribe_url(cfg: &HttpRpcClientConfig, filter: &EventFilter, since: Since) -> String {
    let mut url = format!("{}/events?", cfg.base_url);
    match filter {
        EventFilter::All => url.push_str("scope=all"),
        EventFilter::Job { job_id } => {
            url.push_str("scope=job&job_id=");
            url.push_str(&job_id.to_string());
        }
    }
    if let Some(cursor) = since {
        url.push_str("&since=");
        url.push_str(&cursor.0.to_string());
    }
    if let Some(token) = &cfg.token {
        url.push_str("&token=");
        url.push_str(token);
    }
    url
}
