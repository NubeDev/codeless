use std::convert::Infallible;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use codeless_rpc::{EventFilter, Since};
use codeless_types::{EventCursor, JobId};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use std::time::Duration;

use crate::{auth::constant_time_eq, AppState, AuthMode};

/// Query shape for `GET /events`. The browser builds this in
/// `HttpSseClient::buildSubscribeUrl`; the field names and the
/// `scope`/`job_id` discriminator must match that file.
///
/// `since` is the last seen `EventCursor.0` so the SSE handler can
/// resume after a reconnect without dropping events. `EventSource`
/// also drives reconnects with `Last-Event-ID`, but we accept the
/// query form too so a *fresh* connection (different `EventSource`
/// instance) can still resume — the browser stores the cursor itself.
#[derive(Debug, Deserialize)]
pub(crate) struct EventsQuery {
    pub scope: String,
    pub job_id: Option<JobId>,
    pub since: Option<i64>,
    pub token: Option<String>,
}

pub(crate) async fn events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, String)> {
    if let AuthMode::Required { token } = &state.auth {
        let supplied = q.token.as_deref().unwrap_or("");
        if !constant_time_eq(supplied, token) {
            return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
        }
    }

    let filter = match q.scope.as_str() {
        "all" => EventFilter::All,
        "job" => {
            let job_id = q
                .job_id
                .ok_or((StatusCode::BAD_REQUEST, "job scope requires job_id".into()))?;
            EventFilter::Job { job_id }
        }
        other => {
            return Err((StatusCode::BAD_REQUEST, format!("unknown scope: {other}")));
        }
    };

    // EventSource's automatic reconnect ships `Last-Event-ID` in the
    // request header; a fresh client mint passes `?since=` in the query.
    // The header takes precedence when both are present because it
    // reflects the in-flight cursor the *browser* believes it last saw,
    // which is strictly newer than a stale query value carried over
    // from a closed tab.
    let since: Since = parse_last_event_id(&headers).or(q.since).map(EventCursor);

    let stream = state
        .rpc
        .subscribe(filter, since)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Each `EventEnvelope` becomes one SSE `data:` frame. The
    // envelope's `cursor.0` is also set as the SSE `id` so EventSource
    // populates `Last-Event-ID` on reconnect — the server side of the
    // resume path is the `since` extractor above.
    let sse_stream = stream.map(|item| match item {
        Ok(env) => {
            let cursor = env.cursor.0;
            let data = serde_json::to_string(&env)
                .unwrap_or_else(|_| String::from("{\"error\":\"serialise\"}"));
            Ok(SseEvent::default().id(cursor.to_string()).data(data))
        }
        Err(err) => {
            let data = format!(
                "{{\"error\":{}}}",
                serde_json::Value::String(err.to_string())
            );
            Ok(SseEvent::default().event("error").data(data))
        }
    });

    // Heartbeat every 20 s. Intermediaries (nginx default 60 s,
    // cloudflare 100 s, most NAT-bound corporate proxies anywhere
    // from 30-90 s) silently kill an idle TCP stream — the SSE
    // comment frame keeps it alive and gives the client a
    // "stream is healthy" liveness signal it can time out against
    // when even comments stop arriving.
    Ok(Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("heartbeat"),
    ))
}

/// Parse `Last-Event-ID` per the WHATWG EventSource spec — value is
/// the most recent event id the browser saw. EventSource ships it
/// automatically on auto-reconnect; a custom client may set it
/// manually. Invalid / missing values fall through to the `?since=`
/// query param.
fn parse_last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
}
