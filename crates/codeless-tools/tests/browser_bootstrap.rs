//! Bootstrap-mode integration tests for BrowserManager.
//!
//! Most of the bootstrap pipeline (download Node, npm install,
//! playwright install chromium) needs the network and several
//! minutes — that work is exercised by the opt-in
//! `real_playwright_via_bootstrap` test below, gated on
//! CODELESS_PLAYWRIGHT_HOME being set to a writable directory.
//!
//! What runs in CI: the resolve-cache property (with_bootstrap +
//! resolved_config invariants) and the bootstrap.rs unit tests.

use std::path::PathBuf;
use std::time::Duration;

use codeless_tools::browser::{BootstrapPaths, BrowserManager, BrowserManagerConfig};

#[tokio::test]
async fn bootstrap_mode_starts_with_no_resolved_config() {
    let paths = BootstrapPaths::under_codeless_home(std::path::Path::new("/tmp/nonexistent"));
    let template = BrowserManagerConfig::new(
        PathBuf::from("/never/used"),
        PathBuf::from(""),
        PathBuf::from(""),
    );
    let mgr = BrowserManager::with_bootstrap(paths, template);
    assert!(mgr.resolved_config().await.is_none());
}

#[tokio::test]
async fn resolved_mode_caches_config_immediately() {
    let cfg = BrowserManagerConfig::new(
        PathBuf::from("/usr/bin/bash"),
        PathBuf::from("/tmp/never"),
        PathBuf::from("/tmp/never"),
    );
    let mgr = BrowserManager::new(cfg);
    let cached = mgr.resolved_config().await.expect("cached");
    assert_eq!(cached.node_bin, PathBuf::from("/usr/bin/bash"));
}

/// End-to-end: with CODELESS_PLAYWRIGHT_HOME set to a fresh dir, the
/// manager downloads Node + Playwright + Chromium, opens a real
/// Playwright session, and navigates somewhere. Several minutes on
/// first run; cached re-runs are fast.
#[tokio::test]
#[ignore = "downloads ~250MB; opt-in via CODELESS_PLAYWRIGHT_HOME"]
async fn real_playwright_via_bootstrap() {
    let home = std::env::var_os("CODELESS_PLAYWRIGHT_HOME")
        .map(PathBuf::from)
        .expect("set CODELESS_PLAYWRIGHT_HOME to a writable dir");

    let paths = BootstrapPaths::under_codeless_home(&home);
    let template = BrowserManagerConfig::new(
        // node_bin / sidecar_script / browsers_dir all replaced by
        // bootstrap, but the constructor needs *something*.
        PathBuf::from("/will/be/overwritten"),
        PathBuf::from(""),
        PathBuf::from(""),
    );
    let mgr = BrowserManager::with_bootstrap(paths, template);

    let session = mgr
        .request(
            "session.create",
            serde_json::json!({}),
            Some(Duration::from_secs(60)),
        )
        .await
        .expect("session.create");
    let session_id = session
        .get("session_id")
        .or_else(|| session.get("sessionId"))
        .and_then(|v| v.as_str())
        .expect("session id present")
        .to_string();

    let nav = mgr
        .request(
            "page.goto",
            serde_json::json!({
                "session_id": session_id,
                "url": "data:text/html,<h1>hi</h1>",
                "wait_until": "load",
            }),
            Some(Duration::from_secs(60)),
        )
        .await
        .expect("page.goto");
    assert!(nav.get("page_id").is_some());

    mgr.shutdown().await;
}
