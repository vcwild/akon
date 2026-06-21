//! Backend-agnostic VPN connection boundary
//!
//! This module defines the **durable abstraction** that decouples akon's
//! orchestration logic from *how* a VPN connection is actually established.
//!
//! The production implementation is the native, in-process F5 client
//! ([`crate::vpn::f5::NativeF5Backend`]). The boundary is also implemented by a
//! `SimulatedBackend` test oracle, so the native backend is validated against
//! the exact same scenario suite (cross-backend equivalence).
//!
//! Crucially, the vocabulary here ([`LifecycleEvent`]) is intentionally
//! *backend-agnostic*: it describes connection lifecycle outcomes, not the
//! mechanics of any particular implementation.

use std::net::IpAddr;
use tokio::sync::mpsc::UnboundedReceiver;

/// Credentials handed to a backend to establish a connection.
///
/// The backend is responsible for transmitting these securely (the native F5
/// backend posts the password over TLS). The framework never persists these.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// VPN username.
    pub username: String,
    /// Pre-computed password (e.g. `PIN + OTP`).
    pub password: String,
}

impl Credentials {
    /// Create new credentials.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

/// An opaque handle to a live connection.
///
/// The native backend wraps an internal session identifier. Callers MUST treat
/// it as opaque and not assume it is a PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionHandle(pub u64);

impl ConnectionHandle {
    /// Raw numeric value of the handle (for diagnostics only).
    pub fn raw(&self) -> u64 {
        self.0
    }
}

/// A termination signal to deliver to a connection (used by the test
/// `SimulatedBackend` to model graceful vs. forced teardown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermSignal {
    /// Graceful termination.
    Graceful,
    /// Forced termination.
    Forced,
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The user (or akon) requested disconnect.
    UserRequested,
    /// The server closed the session.
    ServerClosed,
    /// The underlying transport/process terminated unexpectedly.
    LinkLost,
}

impl DisconnectReason {
    /// Whether this disconnect was explicitly requested by the user/akon.
    pub fn is_user_requested(&self) -> bool {
        matches!(self, DisconnectReason::UserRequested)
    }
}

/// Category of a terminal failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// Credentials were rejected.
    Authentication,
    /// Network/transport failure (unreachable server, TLS, etc.).
    Network,
    /// A scripted test backend ran out of steps before a terminal event.
    ScriptExhausted,
    /// Any other backend-internal failure.
    Backend,
}

/// Backend-agnostic, observable events emitted across a connection's lifetime.
///
/// This is the contract surface tests assert on. Ordering follows the state
/// machine documented in `specs/005-test-actors-framework/data-model.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// Connection attempt has begun.
    Connecting,
    /// Authentication is in progress.
    Authenticating,
    /// An authenticated session was established (pre-tunnel).
    SessionEstablished,
    /// The tunnel/interface is configured with an address.
    LinkUp { ip: IpAddr, device: String },
    /// The connection is fully usable.
    Connected { ip: IpAddr, device: String },
    /// The link is believed unhealthy/down (from health checks).
    HealthDegraded,
    /// A reconnection attempt is underway.
    Reconnecting { attempt: u32 },
    /// The connection ended normally.
    Disconnected { reason: DisconnectReason },
    /// The connection failed terminally.
    Failed { kind: FailureKind, detail: String },
}

impl LifecycleEvent {
    /// True if this is a terminal event (no further events expected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LifecycleEvent::Disconnected { .. } | LifecycleEvent::Failed { .. }
        )
    }

    /// Short, stable label for diagnostics/timeline printing.
    pub fn label(&self) -> &'static str {
        match self {
            LifecycleEvent::Connecting => "Connecting",
            LifecycleEvent::Authenticating => "Authenticating",
            LifecycleEvent::SessionEstablished => "SessionEstablished",
            LifecycleEvent::LinkUp { .. } => "LinkUp",
            LifecycleEvent::Connected { .. } => "Connected",
            LifecycleEvent::HealthDegraded => "HealthDegraded",
            LifecycleEvent::Reconnecting { .. } => "Reconnecting",
            LifecycleEvent::Disconnected { .. } => "Disconnected",
            LifecycleEvent::Failed { .. } => "Failed",
        }
    }
}

/// Errors a backend may return from its control methods.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// `connect` was called while already connected.
    #[error("backend is already connected")]
    AlreadyConnected,

    /// A control operation was attempted before connecting.
    #[error("backend is not connected")]
    NotConnected,

    /// The backend failed to start the connection.
    #[error("failed to start connection: {0}")]
    StartFailed(String),

    /// Teardown failed.
    #[error("failed to disconnect: {0}")]
    DisconnectFailed(String),
}

/// The durable VPN backend abstraction.
///
/// Implementations: [`crate::vpn::f5::NativeF5Backend`] (production) and the
/// `SimulatedBackend` test oracle.
///
/// ## Design note: why channel-based, not `async fn`
///
/// `connect` is synchronous and returns a stream ([`UnboundedReceiver`]) of
/// lifecycle events. The backend performs its asynchronous work on internally
/// spawned tasks and pushes events into the channel. This mirrors the existing
/// actor pattern in [`crate::vpn::reconnection`] and avoids pulling in an
/// `async-trait` dependency, keeping the crate dependency-light (in line with
/// the project's goal of eventually shipping with no required dependencies).
pub trait VpnBackend: Send {
    /// Begin establishing a connection.
    ///
    /// Returns a receiver of [`LifecycleEvent`]s. The stream ends after a
    /// terminal event ([`LifecycleEvent::is_terminal`]).
    fn connect(
        &mut self,
        credentials: Credentials,
    ) -> Result<UnboundedReceiver<LifecycleEvent>, BackendError>;

    /// Tear down the connection. Idempotent: calling it on an
    /// already-disconnected backend is a successful no-op.
    fn disconnect(&mut self) -> Result<(), BackendError>;

    /// Whether the connection/tunnel is currently alive.
    fn is_alive(&self) -> bool;

    /// Opaque handle to the live connection, if any.
    fn handle(&self) -> Option<ConnectionHandle>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_events_are_terminal() {
        assert!(LifecycleEvent::Disconnected {
            reason: DisconnectReason::UserRequested
        }
        .is_terminal());
        assert!(LifecycleEvent::Failed {
            kind: FailureKind::Network,
            detail: "x".into()
        }
        .is_terminal());
        assert!(!LifecycleEvent::Connecting.is_terminal());
        assert!(!LifecycleEvent::Connected {
            ip: "10.0.0.1".parse().unwrap(),
            device: "tun0".into()
        }
        .is_terminal());
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(LifecycleEvent::Connecting.label(), "Connecting");
        assert_eq!(
            LifecycleEvent::Reconnecting { attempt: 2 }.label(),
            "Reconnecting"
        );
    }

    #[test]
    fn handle_is_opaque_but_inspectable() {
        let h = ConnectionHandle(42);
        assert_eq!(h.raw(), 42);
    }
}
