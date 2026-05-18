//! Convert `ai_runner::Event` → OpenAI chat-completion-chunk JSON bytes.
//!
//! `ai-ui-core`'s [`SseChunk`](ai_ui_core::SseChunk) wraps the JSON that
//! sits between `data: ` and `\n\n` on the wire; the server adds the SSE
//! framing. We therefore emit each event as the inner JSON body only.
//!
//! Source of truth for the chunk shape is the OpenUI PoC at
//! `codeless/demos/openui-poc/src/main.rs` — see [`tests/parity.rs`] which
//! asserts the byte-for-byte equivalence so the PoC can later be deleted
//! without regressing the wire format.

use ai_runner::{Event, EventKind};
use ai_ui_core::{ProviderError, SseChunk};
use serde_json::json;

/// Translate one upstream runner event into zero or more
/// `Result<SseChunk, ProviderError>` values destined for the client.
///
/// - `Text` → a single `chat.completion.chunk` with `delta.content`.
/// - `Done` → a final `finish_reason = "stop"` chunk followed by
///   [`SseChunk::done`] (the `[DONE]` sentinel).
/// - `Error` → a [`ProviderError::Other`].
/// - `Connected` / `ToolUse` → dropped for the OpenUI text-stream
///   contract; OpenUI consumes plain text deltas only.
pub fn event_to_chunks(chat_id: &str, event: &Event) -> Vec<Result<SseChunk, ProviderError>> {
    match &event.kind {
        EventKind::Text { content } if !content.is_empty() => {
            let chunk = json!({
                "id": chat_id,
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "content": content },
                    "finish_reason": serde_json::Value::Null,
                }],
            });
            vec![Ok(SseChunk::from_json(&chunk))]
        }
        EventKind::Text { .. } => Vec::new(),
        EventKind::Done { .. } => {
            let stop = json!({
                "id": chat_id,
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                }],
            });
            vec![Ok(SseChunk::from_json(&stop)), Ok(SseChunk::done())]
        }
        EventKind::Error { message } => {
            vec![Err(ProviderError::Other(message.clone()))]
        }
        EventKind::Connected { .. } | EventKind::ToolUse { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_runner::SessionId;

    fn ev(kind: EventKind) -> Event {
        Event {
            session_id: SessionId::from("test"),
            provider: "claude".into(),
            kind,
        }
    }

    fn chunk_str(c: &SseChunk) -> String {
        String::from_utf8(c.0.to_vec()).expect("chunk is UTF-8 JSON")
    }

    #[test]
    fn text_event_emits_one_chunk_with_delta_content() {
        let out = event_to_chunks(
            "chatcmpl-test",
            &ev(EventKind::Text {
                content: "hello".into(),
            }),
        );
        assert_eq!(out.len(), 1);
        let body = chunk_str(out[0].as_ref().expect("ok"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["id"], "chatcmpl-test");
        assert_eq!(v["object"], "chat.completion.chunk");
        assert_eq!(v["choices"][0]["delta"]["content"], "hello");
        assert_eq!(v["choices"][0]["finish_reason"], serde_json::Value::Null);
    }

    #[test]
    fn empty_text_event_is_dropped() {
        let out = event_to_chunks(
            "id",
            &ev(EventKind::Text {
                content: String::new(),
            }),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn done_event_emits_finish_stop_then_done_sentinel() {
        let out = event_to_chunks(
            "chatcmpl-test",
            &ev(EventKind::Done {
                duration_ms: 0,
                cost_usd: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            }),
        );
        assert_eq!(out.len(), 2);
        let body = chunk_str(out[0].as_ref().expect("ok"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        // SseChunk::done is the literal sentinel; the server frames it as
        // `data: [DONE]\n\n`.
        assert_eq!(chunk_str(out[1].as_ref().expect("ok")), "[DONE]");
    }

    #[test]
    fn error_event_becomes_provider_error() {
        let out = event_to_chunks(
            "id",
            &ev(EventKind::Error {
                message: "boom".into(),
            }),
        );
        assert_eq!(out.len(), 1);
        let err = out[0].as_ref().expect_err("err");
        match err {
            ProviderError::Other(m) => assert_eq!(m, "boom"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn connected_and_tool_use_are_dropped() {
        assert!(event_to_chunks("id", &ev(EventKind::Connected { model: None })).is_empty());
        assert!(event_to_chunks(
            "id",
            &ev(EventKind::ToolUse {
                id: None,
                name: "fs.read".into(),
                input: None,
            })
        )
        .is_empty());
    }
}
