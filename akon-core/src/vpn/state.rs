//! VPN connection state management
//!
//! Defines the state machine for VPN connection lifecycle and
//! provides thread-safe state tracking.

use std::sync::{Arc, Mutex};

/// VPN connection states
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected
    Disconnected,

    /// Attempting to establish connection
    Connecting,

    /// Successfully connected
    Connected,

    /// Connection failed with an error
    Error(String),

    /// Disconnecting
    Disconnecting,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Disconnected
    }
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionState::Disconnected => write!(f, "disconnected"),
            ConnectionState::Connecting => write!(f, "connecting"),
            ConnectionState::Connected => write!(f, "connected"),
            ConnectionState::Error(msg) => write!(f, "error: {}", msg),
            ConnectionState::Disconnecting => write!(f, "disconnecting"),
        }
    }
}

/// Thread-safe connection state wrapper
#[derive(Debug, Clone)]
pub struct SharedConnectionState(Arc<Mutex<ConnectionState>>);

impl SharedConnectionState {
    /// Create a new shared connection state
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ConnectionState::default())))
    }

    /// Get the current connection state
    pub fn get(&self) -> ConnectionState {
        self.0.lock().unwrap().clone()
    }

    /// Set the connection state
    pub fn set(&self, state: ConnectionState) {
        *self.0.lock().unwrap() = state;
    }

    /// Check if currently connected
    pub fn is_connected(&self) -> bool {
        matches!(self.get(), ConnectionState::Connected)
    }

    /// Check if currently connecting
    pub fn is_connecting(&self) -> bool {
        matches!(self.get(), ConnectionState::Connecting)
    }

    /// Check if in error state
    pub fn is_error(&self) -> bool {
        matches!(self.get(), ConnectionState::Error(_))
    }

    /// Transition to connecting state
    pub fn start_connecting(&self) {
        self.set(ConnectionState::Connecting);
    }

    /// Transition to connected state
    pub fn set_connected(&self) {
        self.set(ConnectionState::Connected);
    }

    /// Transition to disconnected state
    pub fn set_disconnected(&self) {
        self.set(ConnectionState::Disconnected);
    }

    /// Transition to error state
    pub fn set_error(&self, error: String) {
        self.set(ConnectionState::Error(error));
    }

    /// Transition to disconnecting state
    pub fn start_disconnecting(&self) {
        self.set(ConnectionState::Disconnecting);
    }
}

impl Default for SharedConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        let state = SharedConnectionState::new();

        assert_eq!(state.get(), ConnectionState::Disconnected);
        assert!(!state.is_connected());

        state.start_connecting();
        assert_eq!(state.get(), ConnectionState::Connecting);
        assert!(state.is_connecting());

        state.set_connected();
        assert_eq!(state.get(), ConnectionState::Connected);
        assert!(state.is_connected());

        state.set_error("Test error".to_string());
        assert!(state.is_error());
        assert_eq!(state.get(), ConnectionState::Error("Test error".to_string()));

        state.set_disconnected();
        assert_eq!(state.get(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ConnectionState::Disconnected), "disconnected");
        assert_eq!(format!("{}", ConnectionState::Connecting), "connecting");
        assert_eq!(format!("{}", ConnectionState::Connected), "connected");
        assert_eq!(format!("{}", ConnectionState::Disconnecting), "disconnecting");
        assert_eq!(format!("{}", ConnectionState::Error("test".to_string())), "error: test");
    }
}
