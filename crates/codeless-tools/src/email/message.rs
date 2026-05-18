//! RFC 5322 message construction.
//!
//! Deliberately narrow: one text body OR one html body OR both as a
//! `multipart/alternative`. Attachments and inline images are out of
//! scope for the first cut — every backend we plan to support
//! (Gmail REST, SMTP) accepts a raw RFC 5322 blob, so adding
//! multipart/mixed later is a `build()` change with no trait churn.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Mailbox {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            name: None,
        }
    }

    pub fn with_name(address: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            name: Some(name.into()),
        }
    }

    /// Encode as a header value. Display names with non-ASCII or
    /// quote characters are wrapped in MIME encoded-word form so we
    /// never have to reason about header quoting subtleties.
    pub fn encode_header(&self) -> String {
        match &self.name {
            None => self.address.clone(),
            Some(name) if name.is_ascii() && !name.contains(['"', '\\', '<', '>']) => {
                format!("\"{}\" <{}>", name, self.address)
            }
            Some(name) => {
                let b64 = base64_standard(name.as_bytes());
                format!("=?UTF-8?B?{}?= <{}>", b64, self.address)
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Message {
    pub from: Option<Mailbox>,
    #[serde(default)]
    pub to: Vec<Mailbox>,
    #[serde(default)]
    pub cc: Vec<Mailbox>,
    #[serde(default)]
    pub bcc: Vec<Mailbox>,
    pub reply_to: Option<Mailbox>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
}

impl Message {
    /// Render the message as an RFC 5322 byte blob, suitable for
    /// `messages.send` (Gmail) or `DATA` (SMTP).
    pub fn to_rfc5322(&self) -> Result<Vec<u8>, MessageError> {
        if self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() {
            return Err(MessageError::NoRecipients);
        }
        if self.text.is_none() && self.html.is_none() {
            return Err(MessageError::NoBody);
        }

        let mut out = String::new();

        if let Some(from) = &self.from {
            writeln_crlf(&mut out, &format!("From: {}", from.encode_header()));
        }
        if !self.to.is_empty() {
            writeln_crlf(&mut out, &format!("To: {}", join_mailboxes(&self.to)));
        }
        if !self.cc.is_empty() {
            writeln_crlf(&mut out, &format!("Cc: {}", join_mailboxes(&self.cc)));
        }
        // Bcc deliberately not written into the header block — Gmail
        // (and SMTP envelope-level delivery) handles the recipient
        // list separately. We surface bcc through the envelope only.
        if let Some(reply_to) = &self.reply_to {
            writeln_crlf(
                &mut out,
                &format!("Reply-To: {}", reply_to.encode_header()),
            );
        }
        writeln_crlf(&mut out, &format!("Subject: {}", encode_subject(&self.subject)));
        writeln_crlf(&mut out, "MIME-Version: 1.0");

        match (&self.text, &self.html) {
            (Some(text), None) => {
                writeln_crlf(&mut out, "Content-Type: text/plain; charset=UTF-8");
                writeln_crlf(&mut out, "Content-Transfer-Encoding: 8bit");
                writeln_crlf(&mut out, "");
                out.push_str(&normalise_crlf(text));
            }
            (None, Some(html)) => {
                writeln_crlf(&mut out, "Content-Type: text/html; charset=UTF-8");
                writeln_crlf(&mut out, "Content-Transfer-Encoding: 8bit");
                writeln_crlf(&mut out, "");
                out.push_str(&normalise_crlf(html));
            }
            (Some(text), Some(html)) => {
                let boundary = "----=_codeless_alt_boundary_b2f0";
                writeln_crlf(
                    &mut out,
                    &format!(
                        "Content-Type: multipart/alternative; boundary=\"{}\"",
                        boundary
                    ),
                );
                writeln_crlf(&mut out, "");
                let _ = write!(out, "--{}\r\n", boundary);
                writeln_crlf(&mut out, "Content-Type: text/plain; charset=UTF-8");
                writeln_crlf(&mut out, "Content-Transfer-Encoding: 8bit");
                writeln_crlf(&mut out, "");
                out.push_str(&normalise_crlf(text));
                out.push_str("\r\n");
                let _ = write!(out, "--{}\r\n", boundary);
                writeln_crlf(&mut out, "Content-Type: text/html; charset=UTF-8");
                writeln_crlf(&mut out, "Content-Transfer-Encoding: 8bit");
                writeln_crlf(&mut out, "");
                out.push_str(&normalise_crlf(html));
                out.push_str("\r\n");
                let _ = write!(out, "--{}--\r\n", boundary);
            }
            (None, None) => unreachable!("guarded above"),
        }

        Ok(out.into_bytes())
    }

    /// Envelope recipients, used by transports that route by SMTP
    /// envelope (Bcc is included here, but never in the header
    /// block; this matches how every mainstream MTA expects Bcc).
    pub fn envelope_recipients(&self) -> Vec<&str> {
        self.to
            .iter()
            .chain(self.cc.iter())
            .chain(self.bcc.iter())
            .map(|m| m.address.as_str())
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("message has no recipients")]
    NoRecipients,
    #[error("message has neither a text nor an html body")]
    NoBody,
}

fn writeln_crlf(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push_str("\r\n");
}

fn join_mailboxes(list: &[Mailbox]) -> String {
    list.iter()
        .map(Mailbox::encode_header)
        .collect::<Vec<_>>()
        .join(", ")
}

fn encode_subject(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", base64_standard(s.as_bytes()))
    }
}

/// Normalise lone LF / CR into CRLF so user-provided bodies don't
/// accidentally break SMTP framing.
fn normalise_crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            out.push('\r');
            out.push('\n');
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        } else if b == b'\n' {
            out.push('\r');
            out.push('\n');
            i += 1;
        } else {
            out.push(b as char);
            i += 1;
        }
    }
    out
}

fn base64_standard(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_simple_text_message() {
        let msg = Message {
            from: Some(Mailbox::new("a@example.com")),
            to: vec![Mailbox::new("b@example.com")],
            subject: "hello".into(),
            text: Some("hi there".into()),
            ..Default::default()
        };
        let raw = String::from_utf8(msg.to_rfc5322().unwrap()).unwrap();
        assert!(raw.contains("From: a@example.com\r\n"));
        assert!(raw.contains("To: b@example.com\r\n"));
        assert!(raw.contains("Subject: hello\r\n"));
        assert!(raw.contains("Content-Type: text/plain"));
        assert!(raw.ends_with("hi there"));
    }

    #[test]
    fn rejects_empty_recipients() {
        let msg = Message {
            subject: "x".into(),
            text: Some("y".into()),
            ..Default::default()
        };
        assert!(matches!(msg.to_rfc5322(), Err(MessageError::NoRecipients)));
    }

    #[test]
    fn bcc_is_not_in_headers_but_is_in_envelope() {
        let msg = Message {
            to: vec![Mailbox::new("a@example.com")],
            bcc: vec![Mailbox::new("secret@example.com")],
            subject: "x".into(),
            text: Some("y".into()),
            ..Default::default()
        };
        let raw = String::from_utf8(msg.to_rfc5322().unwrap()).unwrap();
        assert!(!raw.to_lowercase().contains("bcc:"));
        assert!(msg.envelope_recipients().contains(&"secret@example.com"));
    }

    #[test]
    fn multipart_alternative_when_both_bodies_present() {
        let msg = Message {
            to: vec![Mailbox::new("a@example.com")],
            subject: "x".into(),
            text: Some("plain".into()),
            html: Some("<p>html</p>".into()),
            ..Default::default()
        };
        let raw = String::from_utf8(msg.to_rfc5322().unwrap()).unwrap();
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("plain"));
        assert!(raw.contains("<p>html</p>"));
    }

    #[test]
    fn non_ascii_name_uses_encoded_word() {
        let m = Mailbox::with_name("a@example.com", "Zoë");
        let h = m.encode_header();
        assert!(h.starts_with("=?UTF-8?B?"));
        assert!(h.ends_with(" <a@example.com>"));
    }
}
