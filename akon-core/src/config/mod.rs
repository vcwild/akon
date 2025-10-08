//! Configuration module
//!
//! Handles loading and saving VPN configuration from TOML files.

use serde::{Deserialize, Serialize};

pub mod toml_config;

/// VPN configuration structure
///
/// Contains all non-sensitive VPN connection parameters.
/// Sensitive data like OTP secrets are stored separately in the keyring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnConfig {
    /// VPN server hostname or IP address
    pub server: String,

    /// VPN server port (default: 443)
    pub port: u16,

    /// Username for VPN authentication
    pub username: String,

    /// Optional realm for multi-realm VPN servers
    pub realm: Option<String>,

    /// Connection timeout in seconds
    pub timeout: Option<u32>,
}

impl VpnConfig {
    /// Create a new VPN configuration
    pub fn new(server: String, port: u16, username: String) -> Self {
        Self {
            server,
            port,
            username,
            realm: None,
            timeout: None,
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Validate server is a valid hostname/IP
        if self.server.is_empty() {
            return Err("Server cannot be empty".to_string());
        }

        // Basic hostname validation
        if !self.server.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') {
            return Err("Server contains invalid characters".to_string());
        }

        // Validate port
        if self.port == 0 {
            return Err("Port cannot be zero".to_string());
        }

        // Validate username
        if self.username.is_empty() {
            return Err("Username cannot be empty".to_string());
        }

        // Validate timeout if provided
        if let Some(timeout) = self.timeout {
            if timeout == 0 {
                return Err("Timeout cannot be zero".to_string());
            }
        }

        Ok(())
    }
}

impl Default for VpnConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            port: 443,
            username: String::new(),
            realm: None,
            timeout: Some(30),
        }
    }
}
