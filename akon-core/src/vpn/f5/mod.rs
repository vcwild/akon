//! Native F5 BIG-IP SSL VPN client (pure-Rust replacement for the openconnect
//! delegation, for the F5 protocol).
//!
//! F5 is **PPP-over-HTTPS**. The implementation is decomposed into independently
//! testable layers, each validated by the test actors framework as ground truth:
//!
//! - [`framing`]: F5 `0xf500|len` pre-PPP encapsulation + the RFC1662 HDLC
//!   variant (escape/unescape + FCS16). Pure, byte-exact.
//! - [`ppp`]: PPP header + LCP/IPCP/IP6CP packet build/parse + the negotiation
//!   state machine that reaches "network up". Pure.
//! - [`auth`]: F5 HTTP auth success/cookie/form logic. Pure.
//! - [`config`]: F5 profile/options XML parsing. Pure.
//! - [`http`]: minimal HTTP/1.1 request build + response parse over a
//!   [`crate::vpn::transport::Transport`].
//! - [`backend`]: [`NativeF5Backend`] orchestrating the layers and implementing
//!   [`crate::vpn::backend::VpnBackend`].
//!
//! Protocol ground truth: openconnect `f5.c` / `ppp.c` (see
//! `specs/006-native-f5-backend/`).

pub mod auth;
pub mod backend;
pub mod config;
pub mod dns;
pub mod framing;
pub mod http;
pub mod ppp;
pub mod teardown;
pub mod tls_transport;

// In-process netlink for rootless TUN/route configuration. Linux-only.
#[cfg(target_os = "linux")]
pub mod netlink;

// Real Linux TUN device (production data plane). Linux-only.
#[cfg(target_os = "linux")]
pub mod tun;

pub use backend::NativeF5Backend;
pub use teardown::{HostTeardownPlan, TeardownReport};
pub use tls_transport::TlsTransport;

/// Errors produced by the native F5 layers.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum F5Error {
    /// A frame on the wire had an unexpected F5 encapsulation magic.
    #[error("unexpected F5 encap magic: {0:#06x} (expected 0xf500)")]
    BadEncapMagic(u16),

    /// A frame was truncated relative to its declared length.
    #[error("truncated frame: need {needed} bytes, have {have}")]
    TruncatedFrame { needed: usize, have: usize },

    /// HDLC frame checksum (FCS16) did not validate.
    #[error("HDLC FCS check failed")]
    HdlcFcsInvalid,

    /// A PPP control packet could not be parsed.
    #[error("malformed PPP packet: {0}")]
    MalformedPpp(String),

    /// HTTP auth did not yield the required F5 session cookies.
    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// The F5 options/profile XML was missing required fields.
    #[error("invalid F5 config: {0}")]
    InvalidConfig(String),

    /// The tunnel-upgrade request did not return a success status.
    #[error("tunnel upgrade rejected: HTTP {0}")]
    TunnelUpgradeRejected(u16),

    /// A malformed HTTP response was received.
    #[error("malformed HTTP response: {0}")]
    MalformedHttp(String),
}
