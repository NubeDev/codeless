//! Tool implementations grouped by family.
//!
//! Cross-cutting helpers (URL parsing, network policy enforcement)
//! live here because every network-touching tool reuses them. The
//! second port (`codeless.http.request`) is what made these worth
//! extracting — Phase 1 had one caller, so duplication was cheaper
//! than abstraction.

pub mod browse_fetch;
pub mod browser_crawl;
pub mod browser_eval;
pub mod browser_interact;
pub mod browser_misc;
pub mod browser_navigate;
pub mod browser_read;
pub mod browser_screenshot;
pub mod browser_session;
pub mod github_issue;
pub mod github_pr;
pub mod gmail_send;
pub mod http_request;
pub mod schedule_create;

pub use browse_fetch::BrowseFetchTool;
pub use browser_crawl::BrowserCrawlTool;
pub use browser_eval::BrowserEvalTool;
pub use browser_interact::{
    BrowserClickTool, BrowserFillTool, BrowserHoverTool, BrowserScrollTool, BrowserTypeTool,
};
pub use browser_misc::{BrowserCookiesTool, BrowserExtractTool, BrowserWaitTool};
pub use browser_navigate::BrowserNavigateTool;
pub use browser_read::BrowserReadTool;
pub use browser_screenshot::BrowserScreenshotTool;
pub use browser_session::{
    BrowserSessionCloseTool, BrowserSessionListTool, BrowserSessionOpenTool,
};
pub use github_issue::GithubIssueTool;
pub use github_pr::GithubPrTool;
pub use gmail_send::GmailSendTool;
pub use http_request::HttpRequestTool;
pub use schedule_create::ScheduleCreateTool;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::policy::NetworkMode;

/// Enforce the per-call network policy: `None` denies; `Allowlist`
/// denies hosts not in the list; `Open` permits.
///
/// `url` is taken alongside `host` only so the error message can
/// echo the original request — `host` alone is what the allowlist
/// check uses.
pub(crate) fn check_network_policy(ctx: &ToolCtx, host: &str, url: &str) -> Result<(), ToolError> {
    match ctx.network_mode() {
        NetworkMode::None => Err(ToolError::denied(format!(
            "network disabled; cannot reach '{}'",
            url
        ))),
        NetworkMode::Allowlist => {
            if ctx.allowlist().allows(host) {
                Ok(())
            } else {
                Err(ToolError::denied(format!(
                    "host '{}' not in allowlist",
                    host
                )))
            }
        }
        NetworkMode::Open => Ok(()),
    }
}

/// Extract the host portion of a URL.
///
/// Stdlib-only — codeless deliberately doesn't pull the `url` crate
/// in for one function. The matching rules (exact host, no scheme,
/// no port) line up with `AllowlistFile::allows` semantics. Userinfo
/// (`user:pw@`) is stripped; query and fragment are stripped; port
/// is stripped.
pub(crate) fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host_with_port = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = host_with_port
        .split('@')
        .next_back()
        .unwrap_or(host_with_port);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_host_strips_scheme_path_port_and_userinfo() {
        assert_eq!(
            url_host("https://example.com/foo"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("http://example.com:8080"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("https://user:pw@example.com/foo"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("https://example.com?q=1#frag"),
            Some("example.com".into())
        );
        assert_eq!(url_host("example.com/foo"), Some("example.com".into()));
        assert_eq!(url_host(""), None);
    }
}
