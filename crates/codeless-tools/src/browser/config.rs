// derived from moxxy-ai/moxxy crates/moxxy-runtime/src/browser/config.rs
//
// Bootstrap-related paths (Node download, Playwright install) are
// deferred to a later sub-tick. This config takes pre-installed Node
// and sidecar paths instead — the caller is responsible for putting
// them in place.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct BrowserManagerConfig {
    /// Absolute path to the Node binary used to launch the sidecar.
    pub node_bin: PathBuf,
    /// Arguments passed before `sidecar_script` on the command line.
    /// Defaults to `--max-old-space-size=512` for real Node; tests
    /// that swap in a non-Node fake clear this.
    pub node_args: Vec<String>,
    /// Absolute path to `sidecar.mjs`.
    pub sidecar_script: PathBuf,
    /// Where Playwright should cache its browser binaries.
    /// Passed through as `PLAYWRIGHT_BROWSERS_PATH`.
    pub browsers_dir: PathBuf,

    /// Default per-call timeout if a caller doesn't supply one.
    pub default_timeout: Duration,
    /// Hard cap on per-call timeout — defends against runaway calls
    /// that would otherwise pin the sidecar indefinitely.
    pub max_timeout: Duration,
    /// Time the sidecar may sit idle before the manager kills it.
    pub idle_timeout: Duration,
}

impl BrowserManagerConfig {
    pub fn new(node_bin: PathBuf, sidecar_script: PathBuf, browsers_dir: PathBuf) -> Self {
        Self {
            node_bin,
            node_args: vec!["--max-old-space-size=512".to_string()],
            sidecar_script,
            browsers_dir,
            default_timeout: Duration::from_secs(30),
            max_timeout: Duration::from_secs(120),
            idle_timeout: Duration::from_secs(300),
        }
    }

    pub fn with_node_args(mut self, args: Vec<String>) -> Self {
        self.node_args = args;
        self
    }
}
