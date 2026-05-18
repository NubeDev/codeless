//! Integration test for the ai-ui surface mounted by `codeless-server`.
//!
//! Builds an `AiUiState` with a fake provider + a one-component manifest,
//! merges the router, and exercises every route through `tower::oneshot`:
//!
//! - `GET  /api/ai-ui/components` → the manifest as JSON.
//! - `GET  /api/ai-ui/skills`     → an empty `{ skills: [] }`.
//! - `POST /api/ai-ui/push`       → `{ delivered_to: 0 }` (no subscribers).
//! - `POST /api/ai-ui/chat`       → SSE bytes with the fake provider's
//!   text delta + finish_reason + `[DONE]`, framed by `data:` / `\n\n`.
//!
//! Also confirms the routes are **absent** when `AppState` is built
//! without `with_ai_ui`, so the existing `routes.rs` test suite is not
//! silently affected.

use std::pin::Pin;
use std::sync::Arc;

use ai_ui_core::{
    AiUiState, ChatStream, Provider, ProviderContext, ProviderError, SkillRegistry, SseChunk,
};
use ai_ui_types::{ChatMessage, ComponentEntry, ComponentManifest};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use codeless_runtime::InProcessRpc;
use codeless_server::{build_router, AppState};
use futures_util::stream;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "test-token-ai-ui";

// ---------------------------------------------------------------------------
// A deterministic in-process provider for the test.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FakeProvider;

impl Provider for FakeProvider {
    fn stream_chat(&self, _ctx: ProviderContext, _messages: Vec<ChatMessage>) -> ChatStream {
        let chunks = vec![
            Ok(SseChunk::from_json(&json!({
                "id": "chatcmpl-fake",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "content": "hi" },
                    "finish_reason": Value::Null,
                }],
            }))),
            Ok(SseChunk::from_json(&json!({
                "id": "chatcmpl-fake",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                }],
            }))),
            Ok::<_, ProviderError>(SseChunk::done()),
        ];
        Pin::from(Box::new(stream::iter(chunks)) as Box<dyn futures_util::Stream<Item = _> + Send>)
    }
}

fn fixture_manifest() -> ComponentManifest {
    ComponentManifest {
        name: "test".into(),
        preamble: "test manifest".into(),
        components: vec![ComponentEntry {
            name: "KpiTile".into(),
            description: "A KPI tile.".into(),
            props: "label: string\nvalue: number".into(),
            example: None,
        }],
    }
}

fn build_ai_ui_state() -> AiUiState {
    AiUiState::builder()
        .component_manifest(fixture_manifest())
        .skills(SkillRegistry::default())
        .provider(FakeProvider)
        .build()
        .expect("AiUiState builds with provider + manifest")
}

async fn fresh_app_with_ai_ui() -> axum::Router {
    let rpc = Arc::new(InProcessRpc::new().await.expect("rpc init"));
    let state = AppState::new(rpc, TOKEN).with_ai_ui(build_ai_ui_state());
    build_router(state)
}

async fn fresh_app_without_ai_ui() -> axum::Router {
    let rpc = Arc::new(InProcessRpc::new().await.expect("rpc init"));
    build_router(AppState::new(rpc, TOKEN))
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn components_route_returns_manifest() {
    let app = fresh_app_with_ai_ui().await;
    let resp = app.oneshot(get("/api/ai-ui/components")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["name"], "test");
    assert_eq!(v["components"][0]["name"], "KpiTile");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skills_route_returns_empty_registry() {
    let app = fresh_app_with_ai_ui().await;
    let resp = app.oneshot(get("/api/ai-ui/skills")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["skills"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_route_returns_zero_when_no_subscribers() {
    let app = fresh_app_with_ai_ui().await;
    let resp = app
        .oneshot(post_json(
            "/api/ai-ui/push",
            json!({ "event": { "type": "ping" } }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["delivered_to"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_route_streams_data_frames_and_done() {
    let app = fresh_app_with_ai_ui().await;
    let resp = app
        .oneshot(post_json(
            "/api/ai-ui/chat",
            json!({ "messages": [{ "role": "user", "content": "hi" }] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "text/event-stream"
    );

    // The fake provider emits 3 chunks; each becomes one `data: ...\n\n`
    // frame. The body is small and bounded, safe to slurp.
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let s = std::str::from_utf8(&body).expect("ascii");
    let frames: Vec<&str> = s.split("\n\n").filter(|f| !f.is_empty()).collect();
    assert_eq!(frames.len(), 3, "expected 3 data frames, got: {s:?}");
    assert!(
        frames[0].starts_with("data: {"),
        "first frame should be JSON, got: {:?}",
        frames[0]
    );
    assert!(
        frames[0].contains("\"content\":\"hi\""),
        "first frame should carry the delta, got: {:?}",
        frames[0]
    );
    assert!(
        frames[1].contains("\"finish_reason\":\"stop\""),
        "second frame should finish, got: {:?}",
        frames[1]
    );
    assert_eq!(
        frames[2], "data: [DONE]",
        "third frame should be the OpenAI sentinel"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_ui_routes_absent_when_state_not_configured() {
    let app = fresh_app_without_ai_ui().await;
    let resp = app.oneshot(get("/api/ai-ui/components")).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "ai-ui routes must not be mounted unless AppState.ai_ui is set"
    );
}
