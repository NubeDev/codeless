//! Network policy types shared by every network-touching tool.
//!
//! Ported in spirit (not in code) from moxxy's `NetworkMode` +
//! `AllowlistFile`. The shapes here are deliberately smaller —
//! codeless is single-tenant (SCOPE R5), so there's no per-agent
//! scoping and no on-disk file format yet. When a real allowlist
//! file is needed, this module gets a `load_from_path` ctor and the
//! shape stays the same.

use std::collections::HashSet;

/// What outbound network traffic a tool is allowed to do.
///
/// `Allowlist` is the only mode that consults `AllowlistFile`;
/// `None` and `Open` are evaluated without it. Tools that don't
/// touch the network ignore this entirely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NetworkMode {
    /// No outbound network. Tools that need network return
    /// `ToolError::Denied`.
    #[default]
    None,
    /// Outbound allowed only to hosts the allowlist permits.
    Allowlist,
    /// Outbound to anywhere.
    Open,
}

/// Allowed-host list consulted in `NetworkMode::Allowlist` mode.
///
/// Hosts are stored verbatim — no scheme, no port. A request to
/// `https://example.com/foo` matches an entry of `example.com`.
/// Wildcards are intentionally not supported yet; a real
/// allowlist with subdomain semantics is a later port.
#[derive(Debug, Clone, Default)]
pub struct AllowlistFile {
    hosts: HashSet<String>,
}

impl AllowlistFile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hosts<I, S>(hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            hosts: hosts.into_iter().map(Into::into).collect(),
        }
    }

    pub fn insert(&mut self, host: impl Into<String>) {
        self.hosts.insert(host.into());
    }

    pub fn allows(&self, host: &str) -> bool {
        self.hosts.contains(host)
    }

    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_mode_defaults_to_none() {
        assert_eq!(NetworkMode::default(), NetworkMode::None);
    }

    #[test]
    fn allowlist_matches_only_exact_hosts() {
        let list = AllowlistFile::with_hosts(["example.com", "api.github.com"]);
        assert!(list.allows("example.com"));
        assert!(list.allows("api.github.com"));
        assert!(!list.allows("example.org"));
        assert!(!list.allows("sub.example.com"));
    }

    #[test]
    fn empty_allowlist_allows_nothing() {
        let list = AllowlistFile::new();
        assert!(list.is_empty());
        assert!(!list.allows("example.com"));
    }
}
