//! Transport and TUN device seams for the native VPN backends.
//!
//! These seams isolate the native F5 backend from real I/O so the protocol
//! logic (auth, config, framing, PPP) can be validated entirely offline by the
//! test actors framework — no real TLS endpoint, no root, no network impact.
//!
//! - [`Transport`]: a bidirectional async byte stream (the TLS socket in
//!   production; an in-memory duplex in tests).
//! - [`TunDevice`]: the OS tunnel interface that receives decapsulated IP
//!   packets (a real `/dev/net/tun` in production; a no-op/recording fake in
//!   tests).

use async_trait::async_trait;
use std::io;

/// A bidirectional, ordered, reliable byte stream.
///
/// This is intentionally a byte stream (not message-oriented): the F5 tunnel
/// runs PPP framing on top, and the HTTP auth phase is a byte protocol too.
/// The production implementation wraps a TLS-over-TCP socket; the test
/// implementation is an in-memory duplex driven by the fake F5 server actor.
#[async_trait]
pub trait Transport: Send {
    /// Write the entire buffer, returning once all bytes are flushed.
    async fn send(&mut self, data: &[u8]) -> io::Result<()>;

    /// Read up to `buf.len()` bytes, returning the number read. A return of
    /// `Ok(0)` indicates the peer closed the stream.
    async fn recv(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Close the transport. Idempotent.
    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Creates fresh [`Transport`] connections on demand.
///
/// Real F5 frontends frequently close the TLS connection between auth/config
/// requests (HTTP/1.0-style or `Connection: close`). The HTTP phase therefore
/// needs to be able to **reconnect** for the next request. A factory abstracts
/// "open a new connection to the same server", with a real TLS implementation in
/// production and an in-memory implementation (backed by the fake F5 server) in
/// tests.
#[async_trait]
pub trait TransportFactory: Send {
    /// Open a new connection to the configured server.
    async fn connect(&self) -> io::Result<Box<dyn Transport>>;
}

/// The OS tunnel interface that ingests/produces raw IP packets.
///
/// In production this is a TUN device requiring `CAP_NET_ADMIN`; in tests it is
/// a recording fake so the orchestration can be validated without root.
#[async_trait]
pub trait TunDevice: Send {
    /// The OS interface name (e.g. `tun0`). May be kernel-assigned, so callers
    /// must not assume `tun0`. Defaults to `"tun0"` for fakes that have no real
    /// interface.
    fn name(&self) -> String {
        "tun0".to_string()
    }

    /// Configure the interface with the negotiated parameters.
    async fn configure(&mut self, config: &TunConfig) -> io::Result<()>;

    /// Inject an inbound IP packet (received from the tunnel) into the OS.
    async fn write_packet(&mut self, packet: &[u8]) -> io::Result<()>;

    /// Read an outbound IP packet (from the OS) destined for the tunnel.
    /// `Ok(0)` indicates the device closed.
    async fn read_packet(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// A persistable record of the host mutations this device made during
    /// [`configure`](Self::configure), so an out-of-process `akon vpn off` can
    /// reconcile the host even if this process is killed. Defaults to an empty
    /// plan (fakes and no-op devices change nothing).
    fn teardown_plan(&self) -> crate::vpn::f5::HostTeardownPlan {
        crate::vpn::f5::HostTeardownPlan::default()
    }
}

/// Negotiated tunnel interface configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunConfig {
    /// Assigned IPv4 address (dotted string).
    pub ipv4: Option<String>,
    /// Interface MTU.
    pub mtu: Option<u32>,
    /// DNS servers (dotted strings).
    pub dns: Vec<String>,
    /// Search domains.
    pub domains: Vec<String>,
    /// Split-include routes (CIDR strings).
    pub routes: Vec<String>,
    /// Full-tunnel mode: route ALL traffic through the tunnel (F5
    /// `UseDefaultGateway0`). When true, a default route via the tun is
    /// installed and the VPN server is exempted via the original gateway.
    pub default_gateway: bool,
    /// The VPN server's IP address (dotted), so full-tunnel mode can keep the
    /// encrypted tunnel's own packets off the tunnel (route them via the
    /// pre-existing default gateway).
    pub server_ip: Option<String>,
}

/// A TUN device that drops all traffic.
///
/// Used when the data plane is established but no OS interface is attached
/// (e.g. control-plane-only tests, or environments without `CAP_NET_ADMIN`).
/// `read_packet` blocks until the device is dropped, so the pump's OS→tunnel
/// direction stays idle without busy-looping; the tunnel→OS direction discards
/// packets. This lets the full connect/teardown lifecycle run without root.
#[derive(Default)]
pub struct NoopTun {
    notify: std::sync::Arc<tokio::sync::Notify>,
}

#[async_trait]
impl TunDevice for NoopTun {
    async fn configure(&mut self, _config: &TunConfig) -> io::Result<()> {
        Ok(())
    }

    async fn write_packet(&mut self, _packet: &[u8]) -> io::Result<()> {
        Ok(())
    }

    async fn read_packet(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        // Never produces OS-originated packets; parks until notified (never).
        self.notify.notified().await;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tun_config_default_is_empty() {
        let c = TunConfig::default();
        assert!(c.ipv4.is_none());
        assert!(c.dns.is_empty());
        assert!(c.routes.is_empty());
    }
}
