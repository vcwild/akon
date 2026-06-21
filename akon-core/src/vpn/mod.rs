//! VPN connection module
//!
//! Native, in-process F5 BIG-IP SSL VPN backend and connection state management.

pub mod state;

// Network interruption detection and automatic reconnection
pub mod health_check;
pub mod reconnection;

// Backend-agnostic connection boundary (durable abstraction).
// Implemented by the native F5 backend and validated by the test actors framework.
pub mod backend;
pub mod transport;

// Native F5 BIG-IP SSL VPN backend (pure-Rust; the only VPN backend).
pub mod f5;

// Test actors framework: simulated backend + in-memory actors.
// Gated out of release builds; available to tests and behind the
// `test-actors` feature.
#[cfg(any(test, feature = "test-actors"))]
pub mod testkit;

// Public re-exports
pub use backend::{
    BackendError, ConnectionHandle, Credentials, DisconnectReason, FailureKind, LifecycleEvent,
    TermSignal, VpnBackend,
};
