//! TCP bind helpers shared across host-only binaries.
//!
//! Binding to `127.0.0.1:0` is the only race-free way to get a free
//! port: the OS picks an unused one atomically at bind time and
//! `local_addr()` reports it back. Any "find a free port first, then
//! bind to it later" pattern has a TOCTOU window where another
//! process can steal the port between the two calls — `bind_tcp`
//! exists so no caller is tempted to roll that.

use std::net::SocketAddr;

/// Bind a TCP listener and return both the listener and the
/// OS-assigned local address.
///
/// `requested = None` binds to loopback with an OS-picked ephemeral
/// port (`127.0.0.1:0`). `requested = Some(addr)` pins the address
/// (and fails with `AddrInUse` if it's already taken — the caller
/// decides whether to fall back).
///
/// The returned [`tokio::net::TcpListener`] is live; hand it to
/// `axum::serve` / `hyper` / whatever transport you're driving. Do
/// not drop it and re-bind to the reported address — the port would
/// briefly be free and another process could grab it.
pub async fn bind_tcp(
    requested: Option<SocketAddr>,
) -> std::io::Result<(tokio::net::TcpListener, SocketAddr)> {
    let addr = requested.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ephemeral_port_is_picked_when_none() {
        let (listener, addr) = bind_tcp(None).await.expect("bind ephemeral");
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0, "OS must assign a real port");
        drop(listener);
    }

    #[tokio::test]
    async fn two_ephemeral_binds_get_distinct_ports() {
        let (l1, a1) = bind_tcp(None).await.unwrap();
        let (l2, a2) = bind_tcp(None).await.unwrap();
        assert_ne!(a1.port(), a2.port());
        drop((l1, l2));
    }

    #[tokio::test]
    async fn pinned_address_round_trips() {
        let (probe, addr) = bind_tcp(None).await.unwrap();
        drop(probe);
        let pinned = SocketAddr::from(([127, 0, 0, 1], addr.port()));
        match bind_tcp(Some(pinned)).await {
            Ok((_l, got)) => assert_eq!(got.port(), pinned.port()),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
