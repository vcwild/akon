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
    /// Pattern for SSL/TLS errors
    ssl_error_pattern: Regex,
    /// Pattern for certificate validation errors
    cert_error_pattern: Regex,
    /// Pattern for TUN device errors
    tun_error_pattern: Regex,
    /// Pattern for DNS resolution errors
    dns_error_pattern: Regex,
}

impl OutputParser {
    /// Create a new OutputParser with compiled regex patterns
    pub fn new() -> Self {
        Self {
            // Match both old format "Connected tun0 as X.X.X.X" and new F5 format "Configured as X.X.X.X"
            tun_configured_pattern: Regex::new(r"(?:Connected\s+(\w+)\s+as|Configured as)\s+(\S+)")
                .expect("Failed to compile tun_configured pattern"),
            established_pattern: Regex::new(
                r"Established connection|SSL connected|with SSL connected",
            )
            .expect("Failed to compile established pattern"),
            auth_failed_pattern: Regex::new(r"Failed to authenticate")
                .expect("Failed to compile auth_failed pattern"),
            post_pattern: Regex::new(r"POST\s+https?://").expect("Failed to compile post pattern"),
            connect_response_pattern: Regex::new(r"Got CONNECT response")
                .expect("Failed to compile connect_response pattern"),
            f5_session_pattern: Regex::new(r"Connected to F5 Session Manager")
                .expect("Failed to compile f5_session pattern"),
            ssl_error_pattern: Regex::new(r"(?i)SSL|TLS|connection failure|handshake")
                .expect("Failed to compile ssl_error pattern"),
            cert_error_pattern: Regex::new(r"(?i)certificate|cert.*invalid|verification failed")
                .expect("Failed to compile cert_error pattern"),
            tun_error_pattern: Regex::new(r"(?i)failed to open tun|tun.*error|no tun device")
                .expect("Failed to compile tun_error pattern"),
            dns_error_pattern: Regex::new(
                r"(?i)cannot resolve|unknown host|name resolution|getaddrinfo failed|Name or service not known"
            )
            .expect("Failed to compile dns_error pattern"),
        }
    }

    /// Parse a line from OpenConnect stdout
    ///
    /// Returns a ConnectionEvent based on the line content
    pub fn parse_line(&self, line: &str) -> ConnectionEvent {
        // Check for TUN configuration - F5 format includes connection confirmation
        // Example: "Configured as 10.10.62.228, with SSL connected and DTLS disabled"
        if let Some(captures) = self.tun_configured_pattern.captures(line) {
            // Group 1 is device (optional for F5 format), Group 2 is IP
            let device = captures
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "tun".to_string()); // Default for F5 format

            // IP is in group 2 for both formats
            let ip_str = captures
                .get(2)
                .or_else(|| captures.get(1)) // Fallback if only one capture group
                .map(|m| m.as_str())
                .unwrap_or("");

            // Extract just the IP address (remove trailing commas, etc.)
            let ip_clean = ip_str.trim_end_matches(',').trim();

            if let Ok(ip) = ip_clean.parse::<IpAddr>() {
                // Check if this line also indicates connection is established (F5 format)
                if line.contains("SSL connected") || line.contains("DTLS") {
                    return ConnectionEvent::Connected { device, ip };
                }
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

        // Check for SSL/TLS errors
        if self.ssl_error_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::NetworkError {
                    reason: "SSL/TLS connection failure".to_string(),
                },
                raw_output: line.to_string(),
            };
        }

        // Check for certificate validation errors
        if self.cert_error_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::NetworkError {
                    reason: "Certificate validation failed".to_string(),
                },
                raw_output: line.to_string(),
            };
        }

        // Check for TUN device errors
        if self.tun_error_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::ConnectionFailed {
                    reason: "Failed to open TUN device - try running with sudo".to_string(),
                },
                raw_output: line.to_string(),
            };
        }

        // Check for DNS resolution errors
        if self.dns_error_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::NetworkError {
                    reason: "DNS resolution failed - check server address".to_string(),
                },
                raw_output: line.to_string(),
            };
        }

        // Treat unrecognized stderr as unknown output
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
