//! Pattern-based parser for OpenConnect CLI output
//!
//! Extracts ConnectionEvents from OpenConnect stdout/stderr using regex patterns

use crate::error::VpnError;
use crate::vpn::ConnectionEvent;
use regex::Regex;
use std::net::IpAddr;

/// Parser for OpenConnect CLI output
pub struct OutputParser {
    /// Pattern for "Connected tun0 as 10.0.1.100"
    tun_configured_pattern: Regex,
    /// Pattern for "Established connection"
    established_pattern: Regex,
    /// Pattern for authentication failures
    auth_failed_pattern: Regex,
    /// Pattern for "POST https://..." (authentication phase)
    post_pattern: Regex,
    /// Pattern for "Got CONNECT response"
    connect_response_pattern: Regex,
    /// Pattern for "Connected to F5 Session Manager"
    f5_session_pattern: Regex,
}

impl OutputParser {
    /// Create a new OutputParser with compiled regex patterns
    pub fn new() -> Self {
        Self {
            tun_configured_pattern: Regex::new(r"Connected\s+(\w+)\s+as\s+(\S+)")
                .expect("Failed to compile tun_configured pattern"),
            established_pattern: Regex::new(r"Established connection")
                .expect("Failed to compile established pattern"),
            auth_failed_pattern: Regex::new(r"Failed to authenticate")
                .expect("Failed to compile auth_failed pattern"),
            post_pattern: Regex::new(r"POST\s+https?://")
                .expect("Failed to compile post pattern"),
            connect_response_pattern: Regex::new(r"Got CONNECT response")
                .expect("Failed to compile connect_response pattern"),
            f5_session_pattern: Regex::new(r"Connected to F5 Session Manager")
                .expect("Failed to compile f5_session pattern"),
        }
    }

    /// Parse a line from OpenConnect stdout
    ///
    /// Returns a ConnectionEvent based on the line content
    pub fn parse_line(&self, line: &str) -> ConnectionEvent {
        // Check for TUN configuration
        if let Some(captures) = self.tun_configured_pattern.captures(line) {
            let device = captures.get(1).map(|m| m.as_str().to_string()).unwrap();
            let ip_str = captures.get(2).map(|m| m.as_str()).unwrap();

            if let Ok(ip) = ip_str.parse::<IpAddr>() {
                return ConnectionEvent::TunConfigured { device, ip };
            }
        }

        // Check for authentication failure
        if self.auth_failed_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::AuthenticationFailed,
                raw_output: line.to_string(),
            };
        }

        // Check for POST (authentication phase)
        if self.post_pattern.is_match(line) {
            return ConnectionEvent::Authenticating {
                message: "Authenticating with server...".to_string(),
            };
        }

        // Check for CONNECT response
        if self.connect_response_pattern.is_match(line) {
            return ConnectionEvent::Authenticating {
                message: "Received server response".to_string(),
            };
        }

        // Check for F5 session establishment
        if self.f5_session_pattern.is_match(line) {
            return ConnectionEvent::F5SessionEstablished {
                session_token: None, // Redacted for security
            };
        }

        // Check for established connection
        if self.established_pattern.is_match(line) {
            return ConnectionEvent::Authenticating {
                message: "Establishing connection...".to_string(),
            };
        }

        // Fallback to unknown output
        ConnectionEvent::UnknownOutput {
            line: line.to_string(),
        }
    }

    /// Parse a line from OpenConnect stderr
    ///
    /// Returns an Error event or UnknownOutput
    pub fn parse_error(&self, line: &str) -> ConnectionEvent {
        // Check for authentication failures
        if self.auth_failed_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::AuthenticationFailed,
                raw_output: line.to_string(),
            };
        }

        // For now, treat all stderr as unknown output
        // Can be extended with more error patterns
        ConnectionEvent::UnknownOutput {
            line: line.to_string(),
        }
    }
}

impl Default for OutputParser {
    fn default() -> Self {
        Self::new()
    }
}
