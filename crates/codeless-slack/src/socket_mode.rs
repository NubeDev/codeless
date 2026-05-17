//! Slack Socket Mode transport. The protocol shape is documented at
//! <https://api.slack.com/apis/socket-mode>; the relevant pieces for
//! this stage are:
//!
//! 1. `POST https://slack.com/api/apps.connections.open` with the
//!    app-level token as the `Authorization: Bearer` header returns a
//!    one-shot `wss_url` to dial. New connections require a new POST;
//!    the URL is not reusable.
//! 2. The WebSocket emits JSON envelopes. The bot acks each one by
//!    sending `{"envelope_id": "..."}` back over the same socket.
//!    Without the ack Slack will retry every 3 seconds and eventually
//!    disconnect the socket.
//! 3. A `disconnect` envelope (reason `warning` for graceful drain,
//!    `refresh_requested` after ~30 minutes of uptime) signals the
//!    client to reopen via step 1.
//!
//! Stage 4 wires the [`Dispatcher`] in between the ack and the drop:
//! every `events_api` envelope is acked first (so Slack stops
//! retrying) and then handed to the dispatcher in a detached task so
//! a slow `chat.postMessage` reply does not stall the next inbound
//! envelope. Without a dispatcher attached (older callers, or tests
//! that only exercise the transport) the envelopes are still acked
//! but their bodies are discarded.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;

use crate::config::SlackConfig;
use crate::dispatcher::{decode_envelope, Dispatcher};

/// Endpoint that mints a single-use `wss_url`. Overridden by tests via
/// `SocketModeSession::with_connect_endpoint`.
const DEFAULT_CONNECT_ENDPOINT: &str = "https://slack.com/api/apps.connections.open";

/// Cap on the reconnect backoff so a sustained Slack outage does not
/// stretch retry intervals into hours.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Initial backoff after the first failed connect. Doubles up to
/// `MAX_BACKOFF` on consecutive failures, resets on success.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum SocketModeError {
    #[error("apps.connections.open transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("apps.connections.open returned HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("apps.connections.open returned ok=false (error={0:?})")]
    SlackApi(Option<String>),
    #[error("malformed wss_url from apps.connections.open: {0}")]
    BadWssUrl(String),
    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
}

/// Wire shape of the `apps.connections.open` response. Slack returns
/// many other fields; only `ok` and `url` are load-bearing here.
#[derive(Debug, Deserialize)]
struct OpenConnectionResponse {
    ok: bool,
    url: Option<String>,
    error: Option<String>,
}

/// Envelope sent back to Slack to ack an inbound message. Slack
/// distinguishes acks from other client-to-server frames purely by the
/// presence of `envelope_id`; the field name has to match exactly.
#[derive(Debug, Serialize)]
struct Ack<'a> {
    envelope_id: &'a str,
}

/// Long-lived Slack Socket Mode session. Owns the config, a shared
/// `reqwest::Client`, and an optional dispatcher. Spawn it via
/// [`crate::SlackBot::spawn`] for production wiring, or call
/// [`SocketModeSession::run_until_shutdown`] directly in tests after
/// wiring a stub endpoint with
/// [`SocketModeSession::with_connect_endpoint`].
pub struct SocketModeSession {
    config: SlackConfig,
    http: Arc<reqwest::Client>,
    connect_endpoint: String,
    dispatcher: Option<Dispatcher>,
}

impl SocketModeSession {
    pub fn new(config: SlackConfig, http: Arc<reqwest::Client>) -> Self {
        Self {
            config,
            http,
            connect_endpoint: DEFAULT_CONNECT_ENDPOINT.to_string(),
            dispatcher: None,
        }
    }

    /// Replace the `apps.connections.open` endpoint. Used by tests to
    /// point the session at a wiremock stub; production callers should
    /// not touch this.
    pub fn with_connect_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.connect_endpoint = endpoint.into();
        self
    }

    /// Attach a dispatcher. Without one the session still acks every
    /// envelope it receives but does not run the command parser
    /// against the body — used by transport-only tests.
    pub fn with_dispatcher(mut self, dispatcher: Dispatcher) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Drive the connect loop until the supplied oneshot fires.
    ///
    /// Each iteration:
    ///   1. POSTs `apps.connections.open` for a fresh `wss_url`.
    ///   2. Dials the WebSocket and pumps frames until the socket
    ///      closes (graceful `disconnect` envelope or transport
    ///      failure).
    ///   3. Sleeps a backoff interval and loops.
    ///
    /// The shutdown signal is consulted inside the WebSocket pump (so
    /// mid-connection exit is prompt) and during the backoff sleep
    /// between attempts (so a signal does not stall server shutdown by
    /// up to `MAX_BACKOFF`). A cheap `try_recv` poll at the top of each
    /// iteration covers the third blocking call — the
    /// `apps.connections.open` request — closing the only remaining
    /// window where the loop could ignore the signal.
    pub async fn run_until_shutdown(self, mut shutdown: oneshot::Receiver<()>) {
        let mut backoff = INITIAL_BACKOFF;
        loop {
            if shutdown_polled(&mut shutdown) {
                tracing::info!("slack: shutdown signal received");
                return;
            }
            match self.connect_once(&mut shutdown).await {
                Ok(ConnectOutcome::Shutdown) => return,
                Ok(ConnectOutcome::Disconnected) => {
                    tracing::info!("slack: socket disconnected; reconnecting");
                    backoff = INITIAL_BACKOFF;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        backoff_ms = backoff.as_millis() as u64,
                        "slack: socket-mode error; will retry",
                    );
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    tracing::info!("slack: shutdown signal received during backoff");
                    return;
                }
                _ = sleep(backoff) => {}
            }
        }
    }

    async fn connect_once(
        &self,
        shutdown: &mut oneshot::Receiver<()>,
    ) -> Result<ConnectOutcome, SocketModeError> {
        let wss_url = self.open_connection().await?;
        tracing::info!("slack: socket mode connection opened");
        self.pump_until_closed(&wss_url, shutdown).await
    }

    async fn open_connection(&self) -> Result<String, SocketModeError> {
        let resp = self
            .http
            .post(&self.connect_endpoint)
            .bearer_auth(&self.config.app_token)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            return Err(SocketModeError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let payload: OpenConnectionResponse = resp.json().await?;
        if !payload.ok {
            return Err(SocketModeError::SlackApi(payload.error));
        }
        let url = payload
            .url
            .ok_or_else(|| SocketModeError::BadWssUrl("response missing `url` field".into()))?;
        // Slack returns the URL with `?ts=...` appended; tokio-tungstenite
        // accepts the full URL but we keep a parse pass so a malformed
        // response surfaces a clear error instead of a generic dial failure.
        url::Url::parse(&url).map_err(|e| SocketModeError::BadWssUrl(e.to_string()))?;
        Ok(url)
    }

    async fn pump_until_closed(
        &self,
        wss_url: &str,
        shutdown: &mut oneshot::Receiver<()>,
    ) -> Result<ConnectOutcome, SocketModeError> {
        let (mut socket, _resp) = tokio_tungstenite::connect_async(wss_url).await?;
        loop {
            tokio::select! {
                biased;
                _ = &mut *shutdown => {
                    let _ = socket.close(None).await;
                    return Ok(ConnectOutcome::Shutdown);
                }
                frame = socket.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(envelope_id) = handle_text_frame(&text, self.dispatcher.as_ref()) {
                            let ack = serde_json::to_string(&Ack { envelope_id: &envelope_id })
                                .expect("Ack serialises");
                            if let Err(e) = socket.send(Message::Text(ack)).await {
                                tracing::warn!(error = %e, "slack: failed to ack envelope");
                                return Err(SocketModeError::WebSocket(e));
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(e) = socket.send(Message::Pong(payload)).await {
                            return Err(SocketModeError::WebSocket(e));
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tracing::info!(?frame, "slack: server closed socket");
                        return Ok(ConnectOutcome::Disconnected);
                    }
                    Some(Ok(_)) => {
                        // Binary / Pong / continuation frames are not
                        // used by Socket Mode; ignore.
                    }
                    Some(Err(e)) => return Err(SocketModeError::WebSocket(e)),
                    None => return Ok(ConnectOutcome::Disconnected),
                }
            }
        }
    }
}

/// Non-blocking peek at a `oneshot::Receiver<()>`. Returns `true` when
/// the sender has fired or been dropped — in either case the caller
/// should treat the channel as terminal and exit the loop. Used to
/// bound the shutdown-detection window around blocking calls that
/// cannot themselves participate in a `tokio::select!` arm without
/// running into the double-borrow that the more obvious
/// "select on shutdown OR connect" shape produces.
fn shutdown_polled(rx: &mut oneshot::Receiver<()>) -> bool {
    use tokio::sync::oneshot::error::TryRecvError;
    match rx.try_recv() {
        Ok(()) => true,
        Err(TryRecvError::Closed) => true,
        Err(TryRecvError::Empty) => false,
    }
}

#[derive(Debug)]
enum ConnectOutcome {
    /// The session shut down because the caller signalled it. The
    /// reconnect loop must exit instead of looping.
    Shutdown,
    /// Slack closed the socket (graceful disconnect / refresh) or the
    /// stream ended. Reconnect after the configured backoff.
    Disconnected,
}

/// Decode a Slack envelope, kick off dispatch (if a dispatcher is
/// attached), and return the envelope id for the caller to ack.
///
/// Dispatch runs in a detached task so a slow `chat.postMessage`
/// reply does not block the next inbound envelope. The ack always
/// races ahead of the dispatch so Slack stops retrying immediately —
/// the SCOPE doc explicitly trades "synchronous reply latency" for
/// "reliable envelope processing" because the operator already sees
/// the action commit in Slack via the eventual reply post.
fn handle_text_frame(text: &str, dispatcher: Option<&Dispatcher>) -> Option<String> {
    let envelope = match decode_envelope(text) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, raw = %text, "slack: failed to decode envelope");
            return None;
        }
    };
    tracing::debug!(kind = ?envelope.kind, "slack: inbound envelope");
    let envelope_id = envelope.envelope_id.clone();
    if let Some(disp) = dispatcher {
        let disp = disp.clone();
        // The envelope decode above clones the parts the dispatcher
        // reads; spawning a detached task here keeps the ack on the
        // hot path and folds the dispatch into the runtime's
        // scheduler. The dispatcher logs and swallows its own
        // failures, so this fire-and-forget is safe.
        tokio::spawn(async move {
            disp.dispatch_envelope(&envelope).await;
        });
    }
    envelope_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_config() -> SlackConfig {
        SlackConfig {
            app_token: "xapp-1-test".to_string(),
            bot_token: "xoxb-test".to_string(),
            channel_id: None,
        }
    }

    #[test]
    fn handle_text_frame_returns_envelope_id_when_present() {
        let frame = r#"{"type":"events_api","envelope_id":"abc-123","payload":{}}"#;
        assert_eq!(handle_text_frame(frame, None).as_deref(), Some("abc-123"));
    }

    #[test]
    fn handle_text_frame_returns_none_for_hello() {
        let frame = r#"{"type":"hello","num_connections":1}"#;
        assert!(handle_text_frame(frame, None).is_none());
    }

    #[test]
    fn handle_text_frame_tolerates_garbage() {
        // Slack will never send this, but a partial/corrupt frame must
        // not crash the pump loop — the warn-and-skip path is
        // exercised here.
        assert!(handle_text_frame("not json", None).is_none());
    }

    #[tokio::test]
    async fn shutdown_signal_breaks_the_connect_loop() {
        // No real Slack endpoint, so the connect call will fail; the
        // important behaviour is that a shutdown signal fired before
        // the next reconnect interval exits the loop cleanly instead
        // of spinning forever.
        let session = SocketModeSession::new(dummy_config(), Arc::new(reqwest::Client::new()))
            .with_connect_endpoint("http://127.0.0.1:1/never-listens");
        let (tx, rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            session.run_until_shutdown(rx).await;
        });
        // Give the first connect attempt a moment to fail, then signal
        // shutdown. The loop must exit during the backoff sleep.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("shutdown completed within deadline")
            .expect("task did not panic");
    }
}
