// GitHub pull-request tool: get / list / create / comment / merge.
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

pub struct GithubPrTool {
    schema: Value,
}

impl GithubPrTool {
    pub fn new() -> Self {
        Self {
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "list", "create", "comment", "merge"],
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
                    "pr_number": {
                        "type": "integer",
                        "description": "Pull-request number. Required for get, comment, and merge."
                    },
                    "title": {
                        "type": "string",
                        "description": "PR title. Required for create."
                    },
                    "body": {
                        "type": "string",
                        "description": "PR description (create) or comment body (comment)."
                    },
                    "head": {
                        "type": "string",
                        "description": "Source branch (e.g. feat/x or owner:feat/x). Required for create."
                    },
                    "base": {
                        "type": "string",
                        "description": "Target branch (e.g. main). Required for create."
                    },
                    "draft": {
                        "type": "boolean",
                        "description": "Open as draft PR. Used with create. Defaults to false."
                    },
                    "state": {
                        "type": "string",
                        "enum": ["open", "closed", "all"],
                        "description": "Filter by state. Used with list. Defaults to open."
                    },
                    "per_page": {
                        "type": "integer",
                        "description": "Results per page for list. Max 100, default 30."
                    },
                    "merge_method": {
                        "type": "string",
                        "enum": ["merge", "squash", "rebase"],
                        "description": "Merge strategy. Used with merge. Defaults to merge."
                    },
                    "commit_title": {
                        "type": "string",
                        "description": "Commit title for the merge commit. Used with merge."
                    }
                },
                "required": ["action", "owner", "repo"]
            }),
        }
    }
}

impl Default for GithubPrTool {
    fn default() -> Self {
        Self::new()
    }
}

fn gh_err(e: octocrab::Error) -> ToolError {
    ToolError::failed(format!("github: {e}"))
}

#[async_trait]
impl Tool for GithubPrTool {
    fn name(&self) -> &str {
        "codeless.github.pr"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        super::check_network_policy(ctx, "api.github.com", "https://api.github.com")?;

        let token =
            std::env::var("GITHUB_TOKEN").map_err(|_| ToolError::denied("GITHUB_TOKEN not set"))?;

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
                let number = pr_number(&args)?;
                let handler = gh.pulls(owner, repo);
                let fut = handler.get(number);
                let pr = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "number": pr.number,
                    "title": pr.title,
                    "state": pr.state,
                    "body": pr.body,
                    "user": pr.user.as_ref().map(|u| u.login.as_str()),
                    "head": pr.head.ref_field,
                    "base": pr.base.ref_field,
                    "draft": pr.draft,
                    "merged": pr.merged,
                    "mergeable": pr.mergeable,
                    "created_at": pr.created_at,
                    "updated_at": pr.updated_at,
                    "html_url": pr.html_url,
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
                let handler = gh.pulls(owner, repo);
                let fut = handler.list().state(state).per_page(per_page).send();
                let page = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                let items: Vec<Value> = page
                    .items
                    .iter()
                    .map(|p| {
                        json!({
                            "number": p.number,
                            "title": p.title,
                            "state": p.state,
                            "draft": p.draft,
                            "user": p.user.as_ref().map(|u| u.login.as_str()),
                            "head": p.head.ref_field,
                            "base": p.base.ref_field,
                            "created_at": p.created_at,
                            "html_url": p.html_url,
                        })
                    })
                    .collect();
                let count = items.len();
                Ok(json!({ "pull_requests": items, "count": count }))
            }

            "create" => {
                let title = args["title"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("create requires 'title'"))?
                    .to_string();
                let head = args["head"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("create requires 'head'"))?
                    .to_string();
                let base = args["base"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("create requires 'base'"))?
                    .to_string();
                let body = args["body"].as_str().unwrap_or("").to_string();
                let draft = args["draft"].as_bool().unwrap_or(false);
                let handler = gh.pulls(owner, repo);
                let fut = handler
                    .create(&title, &head, &base)
                    .body(&body)
                    .draft(draft)
                    .send();
                let pr = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "number": pr.number,
                    "title": pr.title,
                    "html_url": pr.html_url,
                    "draft": pr.draft,
                }))
            }

            "comment" => {
                let number = pr_number(&args)?;
                let body = args["body"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_args("comment requires 'body'"))?
                    .to_string();
                // PR comments share the issues API endpoint.
                let issue_handler = gh.issues(owner, repo);
                let fut = issue_handler.create_comment(number, &body);
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

            "merge" => {
                let number = pr_number(&args)?;
                let method_str = args["merge_method"].as_str().unwrap_or("merge");
                let method = match method_str {
                    "squash" => octocrab::params::pulls::MergeMethod::Squash,
                    "rebase" => octocrab::params::pulls::MergeMethod::Rebase,
                    _ => octocrab::params::pulls::MergeMethod::Merge,
                };
                let commit_title = args["commit_title"].as_str().map(String::from);
                let handler = gh.pulls(owner, repo);
                let mut merge_builder = handler.merge(number).method(method);
                if let Some(ref t) = commit_title {
                    merge_builder = merge_builder.title(t);
                }
                let fut = merge_builder.send();
                let result = select! {
                    biased;
                    _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
                    r = fut => r.map_err(gh_err)?,
                };
                Ok(json!({
                    "merged": result.merged,
                    "sha": result.sha,
                    "message": result.message,
                }))
            }

            other => Err(ToolError::invalid_args(format!("unknown action '{other}'"))),
        }
    }
}

fn pr_number(args: &Value) -> Result<u64, ToolError> {
    args["pr_number"]
        .as_u64()
        .ok_or_else(|| ToolError::invalid_args("this action requires 'pr_number'"))
}
