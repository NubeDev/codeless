//! Headless browser support backed by a Playwright Node sidecar.
//!
//! The Rust side runs a per-host supervised child process that speaks
//! line-delimited JSON-RPC over stdio. The sidecar script and Node
//! binary are caller-provided in this first cut; auto-bootstrap
//! (download Node + Playwright) is a later sub-tick.
//!
//! The sidecar source lives at
//! `crates/codeless-tools/sidecars/playwright/sidecar.mjs` and is
//! ported from moxxy's equivalent. Callers point
//! `BrowserManagerConfig::sidecar_script` at that file on disk.

pub mod bootstrap;
mod config;
mod manager;
mod protocol;
mod sidecar;

pub use bootstrap::{ensure_installed, BootstrapPaths, InstalledSidecar};
pub use config::BrowserManagerConfig;
pub use manager::BrowserManager;
pub use protocol::{RpcError, RpcRequest, RpcResponse};
