// GitHub issue tool: get / list / create / comment.
//
// Auth: reads GITHUB_TOKEN from env at call time. No token = denied.
// Network policy is enforced; callers must allow api.github.com.

use async_trait::async_trait;
use octocrab::Octocrab;
use serde_json::{json, Value};
use tokio::select;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

pub struct GithubIssueTool {
    schema: Value,
}

impl GithubIssueTool {
    pub fn new() -> Self {
        Self {
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "list", "create", "comment"],
                        "description": "Operation to perform."
                    },
                    "owner": {
                        "type": "string",
                        "description": "Repository owner (user or org)."
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository name."
                    },
                    "issue_number": {
                        "type": "integer",
                        "description": "Issue number. Required for get and comment."
                    },
                    "title": {
                        "type": "string",
                        "description": "Issue title. Required for create."
                    },
                    "body": {
                        "type": "string",
                        "description": "Issue body (create) or comment body (comment)."
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Labels to apply. Used with create."
                    },
                    "state": {
                        "type": "string",
                        "enum": ["open", "closed", "all"],
                        "description": "Filter by state. Used with list. Defaults to open."
                    },
                    "per_page": {
                        "type": "integer",
                        "description": "Results per page for list. Max 100, default 30."
                    }
                },
                "required": ["action", "owner", "repo"]
            }),
        }
    }
}

impl Default for GithubIssueTool {
    fn default() -> Self {
        Self::new()
    }
}

fn gh_err(e: octocrab::Error) -> ToolError {
    ToolError::failed(format!("github: {e}"))
}

#[async_trait]
impl Tool for GithubIssueTool {
    fn name(&self) -> &str {
        "codeless.github.issue"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        super::check_network_policy(ctx, "api.github.com", "https://api.github.com")?;

        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| ToolError::denied("GITHUB_TOKEN not set"))?;

        let action = args["action"]
            .as_str()
            .ok_or_else(|| ToolError::invalid_args("missing 'action'"))?;
        let owner = args["owner"]
            .as_str()
            .ok_or_else(|| ToolError::invalid_args("missing 'owner'"))?
            .to_string();
        let repo = args["repo"]
            .as_str()
            .ok_or_else(|| ToolError::invalid_args("missing 'repo'"))?
            .to_string();

        let gh = Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| ToolError::failed(format!("octocrab init: {e}")))?;

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        match action {
            "get" => {
                let number = issue_number(&args)?;
                let handler = gh.issues(owner, repo);
                let fut = handler.get(number);
                let issue = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "number": issue.number,
                    "title": issue.title,
                    "state": format!("{:?}", issue.state),
                    "body": issue.body,
                    "user": issue.user.login,
                    "created_at": issue.created_at,
                    "updated_at": issue.updated_at,
                    "html_url": issue.html_url,
                    "labels": issue.labels.iter().map(|l| &l.name).collect::<Vec<_>>(),
                }))
            }

            "list" => {
                let state_str = args["state"].as_str().unwrap_or("open");
                let per_page = args["per_page"].as_u64().unwrap_or(30).min(100) as u8;
                let state = match state_str {
                    "closed" => octocrab::params::State::Closed,
                    "all" => octocrab::params::State::All,
                    _ => octocrab::params::State::Open,
                };
                let handler = gh.issues(owner, repo);
                let fut = handler.list().state(state).per_page(per_page).send();
                let page = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                let items: Vec<Value> = page
                    .items
                    .iter()
                    .map(|i| json!({
                        "number": i.number,
                        "title": i.title,
                        "state": format!("{:?}", i.state),
                        "user": i.user.login,
                        "created_at": i.created_at,
                        "html_url": i.html_url,
                        "labels": i.labels.iter().map(|l| &l.name).collect::<Vec<_>>(),
                    }))
                    .collect();
                let count = items.len();
                Ok(json!({ "issues": items, "count": count }))
            }

            "create" => {
                let title = args["title"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("create requires 'title'"))?
                    .to_string();
                let body = args["body"].as_str().unwrap_or("").to_string();
                let labels: Vec<String> = args["labels"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let handler = gh.issues(owner, repo);
                let fut = handler.create(&title).body(&body).labels(labels).send();
                let issue = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "number": issue.number,
                    "title": issue.title,
                    "html_url": issue.html_url,
                }))
            }

            "comment" => {
                let number = issue_number(&args)?;
                let body = args["body"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("comment requires 'body'"))?
                    .to_string();
                let handler = gh.issues(owner, repo);
                let fut = handler.create_comment(number, &body);
                let comment = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "id": comment.id,
                    "html_url": comment.html_url,
                }))
            }

            other => Err(ToolError::invalid_args(format!("unknown action '{other}'"))),
        }
    }
}

fn issue_number(args: &Value) -> Result<u64, ToolError> {
    args["issue_number"]
        .as_u64()
        .ok_or_else(|| ToolError::invalid_args("this action requires 'issue_number'"))
}
