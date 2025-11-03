//! Connection event types for VPN lifecycle state machine
//!
//! Defines events emitted during OpenConnect CLI connection lifecycle

use crate::error::VpnError;
use std::net::IpAddr;

/// Events emitted during OpenConnect CLI connection lifecycle
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// OpenConnect process started successfully
    ProcessStarted { pid: u32 },

    /// Authentication phase in progress
    Authenticating { message: String },

    /// F5 session manager connection established
    F5SessionEstablished {
        session_token: Option<String>, // May be redacted for security
    },

    /// TUN device configured with assigned IP
    TunConfigured { device: String, ip: IpAddr },

    /// Full VPN connection established
    Connected { ip: IpAddr, device: String },

    /// Connection disconnected normally
    Disconnected { reason: DisconnectReason },

    /// Error occurred during connection
    Error {
        kind: VpnError,
        raw_output: String,
    },

    /// Unparsed output line (fallback)
    UnknownOutput { line: String },
}

/// Reasons for disconnection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    UserRequested,
    ServerDisconnect,
    ProcessTerminated,
    Timeout,
}

/// Internal connection state
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Authenticating,
    Established { ip: IpAddr, device: String },
    Disconnecting,
    Failed { error: String },
}
