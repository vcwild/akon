//! Type definitions and wrappers for secure data handling
//!
//! This module provides type-safe wrappers for sensitive data using the
//! secrecy crate to prevent accidental exposure in logs or debug output.

use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};

/// Wrapper for OTP secrets stored in GNOME Keyring
///
/// This type ensures OTP secrets are never accidentally logged or exposed
/// in debug output, maintaining security throughout the application.
#[derive(Clone, Debug)]
pub struct OtpSecret(Secret<String>);

impl OtpSecret {
    /// Create a new OtpSecret from a Base32-encoded string
    pub fn new(secret: String) -> Self {
        Self(Secret::new(secret))
    }

    /// Expose the secret value (use with caution!)
    ///
    /// This should only be called when absolutely necessary,
    /// such as when passing to cryptographic functions.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// Validate that the secret is valid Base32
    pub fn validate_base32(&self) -> Result<(), crate::error::OtpError> {
        // Basic validation - check for valid Base32 characters
        let secret = self.expose();
        if secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '=' || c == '/')
        {
            Ok(())
        } else {
            Err(crate::error::OtpError::InvalidBase32)
        }
    }
}

impl From<String> for OtpSecret {
    fn from(secret: String) -> Self {
        Self::new(secret)
    }
}

/// Wrapper for generated TOTP tokens
///
/// Generated OTP tokens should also be treated as sensitive data
/// and never logged, even though they have a short lifetime.
#[derive(Clone, Debug)]
pub struct TotpToken(Secret<String>);

impl TotpToken {
    /// Create a new TotpToken from a generated token string
    pub fn new(token: String) -> Self {
        Self(Secret::new(token))
    }

    /// Expose the token value (use with caution!)
    ///
    /// This should only be called when sending the token to stdout
    /// or passing to external systems.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<String> for TotpToken {
    fn from(token: String) -> Self {
        Self::new(token)
    }
}

/// Connection state for VPN operations
///
/// Tracks the current state of the VPN connection with associated metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    /// Not connected to VPN
    Disconnected,
    /// Attempting to establish connection
    Connecting,
    /// Successfully connected to VPN
    Connected {
        /// When the connection was established
        connected_at: std::time::SystemTime,
        /// Server endpoint
        server: String,
    },
    /// Connection failed
    Error {
        /// Error message
        message: String,
    },
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Disconnected
    }
}

/// Keyring entry metadata
///
/// Information about a credential stored in the GNOME Keyring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyringEntry {
    /// Service name (should be "akon-vpn-otp")
    pub service: String,
    /// Username/account identifier
    pub username: String,
    /// When the entry was created/modified
    pub created: std::time::SystemTime,
}

/// IPC message types for daemon communication
///
/// Messages sent between the CLI and background daemon process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Request current connection status
    StatusRequest,
    /// Response with current connection state
    StatusResponse(ConnectionState),
    /// Request to establish VPN connection
    ConnectRequest {
        /// VPN server URL
        server: String,
        /// Username
        username: String,
    },
    /// Response to connection request
    ConnectResponse(Result<(), String>),
    /// Request to disconnect VPN
    DisconnectRequest,
    /// Response to disconnect request
    DisconnectResponse(Result<(), String>),
    /// Shutdown daemon
    Shutdown,
}

/// Result type alias for IPC operations
pub type IpcResult<T> = Result<T, String>;
