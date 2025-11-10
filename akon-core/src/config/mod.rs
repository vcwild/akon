//! Configuration module
//!
//! Handles loading and saving VPN configuration from TOML files.

use serde::{Deserialize, Serialize};

pub mod toml_config;

/// VPN protocol type
///
/// Supported VPN protocols for OpenConnect
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VpnProtocol {
    /// Cisco AnyConnect SSL VPN
    AnyConnect,
    /// Palo Alto Networks GlobalProtect
    GlobalProtect,
    /// Juniper Network Connect
    NC,
    /// Pulse Connect Secure
    Pulse,
    /// F5 Big-IP SSL VPN (default)
    #[default]
    F5,
    /// Fortinet FortiGate SSL VPN
    Fortinet,
    /// Array Networks SSL VPN
    Array,
}

impl VpnProtocol {
    /// Get the protocol name as expected by OpenConnect
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AnyConnect => "anyconnect",
            Self::GlobalProtect => "gp",
            Self::NC => "nc",
            Self::Pulse => "pulse",
            Self::F5 => "f5",
            Self::Fortinet => "fortinet",
            Self::Array => "array",
        }
    }
}

/// VPN configuration structure
///
/// Contains all non-sensitive VPN connection parameters.
/// Sensitive data like OTP secrets are stored separately in the keyring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnConfig {
    /// VPN server hostname or IP address
    pub server: String,

    /// Username for VPN authentication
    pub username: String,

    /// VPN protocol to use (default: AnyConnect)
    #[serde(default)]
    pub protocol: VpnProtocol,

    /// Connection timeout in seconds
    pub timeout: Option<u32>,

    /// Disable DTLS (Datagram TLS) and use only TCP/TLS
    #[serde(default)]
    pub no_dtls: bool,

    /// Enable lazy mode - running akon without arguments connects to VPN
    #[serde(default)]
    pub lazy_mode: bool,
}

impl VpnConfig {
    /// Create a new VPN configuration
    pub fn new(server: String, username: String) -> Self {
        Self {
            server,
            username,
            protocol: VpnProtocol::default(),
            timeout: None,
            no_dtls: false,
            lazy_mode: false,
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Validate server is a valid hostname/IP
        if self.server.is_empty() {
            return Err("Server cannot be empty".to_string());
        }

        // Basic hostname validation
        if !self
            .server
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        {
            return Err("Server contains invalid characters".to_string());
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
            username: String::new(),
            protocol: VpnProtocol::default(),
            timeout: Some(30),
            no_dtls: false,
            lazy_mode: false,
        }
    }
}
