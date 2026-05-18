// Gmail send tool. Thin schema wrapper over `crate::email::GmailMailer`.
//
// Auth: reads GMAIL_ACCESS_TOKEN from env at call time. No token =
// denied. GMAIL_USER_ID overrides the default "me". Network policy
// is enforced; callers must allow gmail.googleapis.com.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::select;

use crate::ctx::ToolCtx;
use crate::email::{GmailMailer, Mailbox, Mailer, Message};
use crate::error::ToolError;
use crate::tool::Tool;

pub struct GmailSendTool {
    schema: Value,
}

impl GmailSendTool {
    pub fn new() -> Self {
        let mailbox_schema = json!({
            "type": "object",
            "properties": {
                "address": { "type": "string" },
                "name": { "type": "string" }
            },
            "required": ["address"]
        });
        Self {
            schema: json!({
                "type": "object",
                "properties": {
                    "from":     mailbox_schema,
                    "to":       { "type": "array", "items": mailbox_schema, "minItems": 1 },
                    "cc":       { "type": "array", "items": mailbox_schema },
                    "bcc":      { "type": "array", "items": mailbox_schema },
                    "reply_to": mailbox_schema,
                    "subject":  { "type": "string" },
                    "text":     { "type": "string", "description": "Plain-text body." },
                    "html":     { "type": "string", "description": "HTML body. Combine with 'text' for multipart/alternative." }
                },
                "required": ["to", "subject"]
            }),
        }
    }
}

impl Default for GmailSendTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GmailSendTool {
    fn name(&self) -> &str {
        "codeless.gmail.send"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        super::check_network_policy(
            ctx,
            "gmail.googleapis.com",
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/send",
        )?;

        let token = std::env::var("GMAIL_ACCESS_TOKEN")
            .map_err(|_| ToolError::denied("GMAIL_ACCESS_TOKEN not set"))?;
        let user_id = std::env::var("GMAIL_USER_ID").unwrap_or_else(|_| "me".to_string());

        let message = parse_message(&args)?;

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let mailer = GmailMailer::new(token, user_id);
        let outcome = select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
            r = mailer.send(&message) => r.map_err(|e| ToolError::failed(format!("gmail send: {e}")))?,
        };

        Ok(json!({
            "message_id": outcome.message_id,
            "backend": outcome.backend,
        }))
    }
}

fn parse_message(args: &Value) -> Result<Message, ToolError> {
    let subject = args
        .get("subject")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid_args("missing 'subject'"))?
        .to_string();
    let to = parse_mailbox_array(args.get("to"), "to")?;
    if to.is_empty() {
        return Err(ToolError::invalid_args("'to' must have at least one entry"));
    }
    let cc = parse_mailbox_array(args.get("cc"), "cc")?;
    let bcc = parse_mailbox_array(args.get("bcc"), "bcc")?;
    let from = parse_optional_mailbox(args.get("from"), "from")?;
    let reply_to = parse_optional_mailbox(args.get("reply_to"), "reply_to")?;

    let text = args.get("text").and_then(Value::as_str).map(str::to_string);
    let html = args.get("html").and_then(Value::as_str).map(str::to_string);
    if text.is_none() && html.is_none() {
        return Err(ToolError::invalid_args("provide 'text' or 'html'"));
    }

    Ok(Message {
        from,
        to,
        cc,
        bcc,
        reply_to,
        subject,
        text,
        html,
    })
}

fn parse_mailbox_array(value: Option<&Value>, field: &str) -> Result<Vec<Mailbox>, ToolError> {
    let Some(v) = value else { return Ok(vec![]) };
    let arr = v
        .as_array()
        .ok_or_else(|| ToolError::invalid_args(format!("'{field}' must be an array")))?;
    arr.iter()
        .map(|item| parse_mailbox(item, field))
        .collect()
}

fn parse_optional_mailbox(
    value: Option<&Value>,
    field: &str,
) -> Result<Option<Mailbox>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => parse_mailbox(v, field).map(Some),
    }
}

fn parse_mailbox(value: &Value, field: &str) -> Result<Mailbox, ToolError> {
    let obj = value
        .as_object()
        .ok_or_else(|| ToolError::invalid_args(format!("'{field}' entry must be an object")))?;
    let address = obj
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid_args(format!("'{field}' entry missing 'address'")))?
        .to_string();
    let name = obj.get("name").and_then(Value::as_str).map(str::to_string);
    Ok(Mailbox { address, name })
}
