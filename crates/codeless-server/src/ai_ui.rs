//! ai-ui surface — `/api/ai-ui/{chat,push,events,skills,components}`.
//!
//! Mounted by `routes::router` only when [`AppState::ai_ui`] is
//! populated. The router has its own `with_state(AiUiState)` so the
//! handlers never have to unwrap the option themselves — if the server
//! was built without an `AiUiState`, these routes are simply absent.
//!
//! Routes are exposed **outside** the bearer middleware so the OpenUI
//! frontend (which has no concept of codeless's bearer token) can call
//! them directly. Deployments that need auth in front of these routes
//! should put a reverse proxy in front of the codeless-server bind.
//! Tracked in the slice-2 summary; the loopback-bind default (R5)
//! keeps single-tenant MVPs safe.

use std::{convert::Infallible, time::Duration};

use ai_ui_core::{AiUiState, ProviderContext};
use ai_ui_types::{ChatRequest, ComponentManifest, PushEvent, PushRequest};
use axum::{
    body::Body,
    extract::State,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use bytes::{Bytes, BytesMut};
use futures_util::stream::{self, Stream, StreamExt};
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;

/// Build the ai-ui sub-router. Caller merges it into the top-level
/// `Router` in `routes::router`.
pub(crate) fn router(state: AiUiState) -> Router {
    Router::new()
        .route("/api/ai-ui/chat", post(chat_handler))
        .route("/api/ai-ui/push", post(push_handler))
        .route("/api/ai-ui/events", get(events_handler))
        .route("/api/ai-ui/skills", get(skills_handler))
        .route("/api/ai-ui/components", get(components_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// /api/ai-ui/chat
// ---------------------------------------------------------------------------

async fn chat_handler(
    State(state): State<AiUiState>,
    Json(payload): Json<ChatRequest>,
) -> Response {
    let prompt = if payload.skills.is_empty() {
        state.prompt().build()
    } else {
        ai_ui_prompt::PromptBuilder::new()
            .components(state.manifest().clone())
            .skills_subset(state.skills(), &payload.skills)
            .build()
    };

    let upstream = state.provider().stream_chat(
        ProviderContext {
            system_prompt: prompt,
        },
        payload.messages,
    );

    // Frame each provider chunk as `data: <bytes>\n\n`. Errors mid-
    // stream go out as a `data:` line so the client surfaces them
    // instead of seeing the connection close silently — mirrors
    // ai-ui-axum's behaviour exactly so a parity test against
    // openui-poc would compare like-for-like bytes.
    let sse_stream = upstream.map(|chunk_result| -> Result<Bytes, Infallible> {
        match chunk_result {
            Ok(chunk) => {
                let mut buf = BytesMut::with_capacity(chunk.0.len() + 8);
                buf.extend_from_slice(b"data: ");
                buf.extend_from_slice(&chunk.0);
                buf.extend_from_slice(b"\n\n");
                Ok(buf.freeze())
            }
            Err(e) => {
                let err = json!({ "error": format!("{e}") });
                Ok(Bytes::from(format!("data: {}\n\n", err)))
            }
        }
    });

    Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache, no-transform")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(sse_stream))
        .expect("static headers are valid")
}

// ---------------------------------------------------------------------------
// /api/ai-ui/push  +  /api/ai-ui/events
// ---------------------------------------------------------------------------

async fn push_handler(
    State(state): State<AiUiState>,
    Json(payload): Json<PushRequest>,
) -> Response {
    let n = state.broadcast(payload.event);
    Json(json!({ "delivered_to": n })).into_response()
}

async fn events_handler(
    State(state): State<AiUiState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|res| async move { res.ok() })
        .map(|event: PushEvent| {
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
            Ok::<_, Infallible>(SseEvent::default().event("push").data(data))
        });
    // Emit a `ready` event first so a freshly connected client knows
    // the SSE handshake completed before any push arrives.
    let stream =
        stream::once(async { Ok::<_, Infallible>(SseEvent::default().event("ready").data("{}")) })
            .chain(stream);
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

// ---------------------------------------------------------------------------
// /api/ai-ui/skills  +  /api/ai-ui/components
// ---------------------------------------------------------------------------

async fn skills_handler(State(state): State<AiUiState>) -> Json<serde_json::Value> {
    let summaries: Vec<_> = state
        .skills()
        .skills()
        .iter()
        .map(|s| s.manifest.summary())
        .collect();
    Json(json!({ "skills": summaries }))
}

async fn components_handler(State(state): State<AiUiState>) -> Json<ComponentManifest> {
    Json(state.manifest().clone())
}
