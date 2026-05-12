//! Minimal `text/event-stream` parser for the `/events` SSE
//! endpoint. The browser uses `EventSource` which hides the spec;
//! here we follow the same shape `codeless-server::sse` emits:
//! one frame per event with `id: <cursor>` and `data: <json>` lines
//! terminated by a blank line. Comments (lines starting with `:`)
//! are SSE keep-alives and are silently dropped.
//!
//! Scope is deliberately tight: this is a single-stream parser, not
//! a general-purpose SSE library. It does *not* implement automatic
//! reconnection — callers handle that by re-invoking `subscribe`
//! with the last seen cursor (`Since::Some(cursor)`). The runtime's
//! own `EventBus::subscribe_since` replays the gap on the server
//! side, so caller-driven reconnect is gap-free.

use codeless_rpc::RpcError;
use codeless_types::EventEnvelope;

/// State carried across body chunks. SSE frames are arbitrary-sized
/// and can straddle TCP/TLS chunk boundaries, so the parser buffers
/// until it sees `\n\n` (or `\r\n\r\n`).
#[derive(Default)]
pub(crate) struct SseParser {
    buf: String,
    current_data: String,
}

impl SseParser {
    /// Push raw bytes into the buffer. Returns zero or more fully
    /// parsed events. Malformed JSON inside a `data:` field is
    /// surfaced as `RpcError::Internal` so the caller sees a clear
    /// error rather than a silent drop.
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Result<EventEnvelope, RpcError>> {
        // SSE is required to be UTF-8. A malformed sequence is
        // unrecoverable mid-frame; surface as a parse error rather
        // than corrupt subsequent frames.
        let text = match std::str::from_utf8(chunk) {
            Ok(s) => s,
            Err(_) => {
                return vec![Err(RpcError::Internal(
                    "sse: non-utf8 byte in event stream".into(),
                ))];
            }
        };
        self.buf.push_str(text);

        let mut out = Vec::new();
        // SSE frames end at the first blank line. Handle both `\n\n`
        // and `\r\n\r\n` — `codeless-server` uses bare `\n` but real
        // proxies (and the browser's own EventSource) tolerate CRLF.
        while let Some(end) = find_blank_line(&self.buf) {
            // `..end` is the frame; `..end_skip` consumes the blank
            // line itself.
            let (frame, rest_start) = split_frame(&self.buf, end);
            out.extend(self.parse_frame(&frame));
            self.buf.drain(..rest_start);
        }
        out
    }

    fn parse_frame(&mut self, frame: &str) -> Vec<Result<EventEnvelope, RpcError>> {
        self.current_data.clear();
        let mut event_name: Option<&str> = None;
        for line in frame.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() || line.starts_with(':') {
                // Blank or comment — skip. `id:` lines are also
                // technically legal here but the cursor is already
                // inside the JSON payload, so we let it pass.
                continue;
            }
            if let Some(value) = strip_field(line, "data") {
                if !self.current_data.is_empty() {
                    self.current_data.push('\n');
                }
                self.current_data.push_str(value);
            } else if let Some(value) = strip_field(line, "event") {
                event_name = Some(value);
            }
            // `id:` and unknown fields are ignored: the cursor we
            // care about is `EventEnvelope.cursor`, sourced from the
            // payload itself; the SSE `id:` is for browser-side
            // `Last-Event-ID` only.
        }

        if event_name == Some("error") {
            return vec![Err(RpcError::Internal(format!(
                "sse: server-side error: {}",
                self.current_data
            )))];
        }
        if self.current_data.is_empty() {
            return Vec::new();
        }
        match serde_json::from_str::<EventEnvelope>(&self.current_data) {
            Ok(env) => vec![Ok(env)],
            Err(e) => vec![Err(RpcError::Internal(format!(
                "sse: bad EventEnvelope JSON: {e}"
            )))],
        }
    }
}

/// Returns the byte offset of the start of the blank-line delimiter
/// (the first `\n` of `\n\n` or `\r\n\r\n`), or `None` if no full
/// frame is buffered.
fn find_blank_line(s: &str) -> Option<usize> {
    if let Some(i) = s.find("\n\n") {
        return Some(i);
    }
    s.find("\r\n\r\n")
}

/// Returns `(frame_text_up_to_blank, byte_offset_after_blank)`.
fn split_frame(s: &str, blank_start: usize) -> (String, usize) {
    if s[blank_start..].starts_with("\r\n\r\n") {
        (s[..blank_start].to_string(), blank_start + 4)
    } else {
        (s[..blank_start].to_string(), blank_start + 2)
    }
}

/// SSE-spec parse of one `field: value` line. The space after the
/// colon is optional and gets stripped if present. Returns `None`
/// when `line` is not the requested field.
fn strip_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?;
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_frame() {
        let mut p = SseParser::default();
        let raw = r#"id: 1
data: {"cursor":1,"job_id":null,"stage_id":null,"task_id":null,"created_at":42,"event":{"type":"repo-added","repo_id":"01J0Y0XK4PC6V8M0H0N6MAW7TR"}}

"#;
        let out = p.feed(raw.as_bytes());
        assert_eq!(out.len(), 1);
        let env = out.into_iter().next().unwrap().unwrap();
        assert_eq!(env.cursor.0, 1);
    }

    #[test]
    fn parses_two_frames_split_across_chunks() {
        let mut p = SseParser::default();
        let chunk_a = r#"id: 1
data: {"cursor":1,"job_id":null,"stage_id":null,"task_id":null,"created_at":1,"event":{"type":"repo-added","repo_id":"01J0Y0XK4PC6V8M0H0N6MAW7TR"}}

id: 2
da"#;
        let chunk_b = r#"ta: {"cursor":2,"job_id":null,"stage_id":null,"task_id":null,"created_at":2,"event":{"type":"repo-removed","repo_id":"01J0Y0XK4PC6V8M0H0N6MAW7TR"}}

"#;
        let first = p.feed(chunk_a.as_bytes());
        assert_eq!(first.len(), 1);
        let second = p.feed(chunk_b.as_bytes());
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn comment_lines_are_dropped() {
        let mut p = SseParser::default();
        let raw = r#": keep-alive
id: 1
data: {"cursor":1,"job_id":null,"stage_id":null,"task_id":null,"created_at":1,"event":{"type":"repo-added","repo_id":"01J0Y0XK4PC6V8M0H0N6MAW7TR"}}

"#;
        let out = p.feed(raw.as_bytes());
        assert_eq!(out.len(), 1);
        out.into_iter().next().unwrap().unwrap();
    }

    #[test]
    fn server_error_event_surfaces_as_rpc_internal() {
        let mut p = SseParser::default();
        let raw = "event: error\ndata: {\"error\":\"boom\"}\n\n";
        let out = p.feed(raw.as_bytes());
        assert!(matches!(out[0], Err(RpcError::Internal(_))));
    }

    #[test]
    fn crlf_frame_delimiter_works() {
        let mut p = SseParser::default();
        let raw = "id: 1\r\ndata: {\"cursor\":1,\"job_id\":null,\"stage_id\":null,\"task_id\":null,\"created_at\":1,\"event\":{\"type\":\"repo-added\",\"repo_id\":\"01J0Y0XK4PC6V8M0H0N6MAW7TR\"}}\r\n\r\n";
        let out = p.feed(raw.as_bytes());
        assert_eq!(out.len(), 1);
        out.into_iter().next().unwrap().unwrap();
    }
}
