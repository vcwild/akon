//! Error types for the akon VPN CLI tool
//!
//! This module defines all error types used throughout the application,
//! providing consistent error handling and user-friendly error messages.

use thiserror::Error;

/// Main error type for the akon application
#[derive(Error, Debug)]
pub enum AkonError {
    /// Errors related to configuration loading/parsing
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// Errors related to keyring operations
    #[error("Keyring error: {0}")]
    Keyring(#[from] KeyringError),

    /// Errors related to VPN connection operations
    #[error("VPN error: {0}")]
    Vpn(#[from] VpnError),

    /// Errors related to OTP/TOTP operations
    #[error("OTP error: {0}")]
    Otp(#[from] OtpError),

    /// Generic I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// TOML parsing errors
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML serialization errors
    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

/// Configuration-related errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load configuration file: {path}")]
    LoadFailed { path: String },

    #[error("Failed to save configuration file: {path}")]
    SaveFailed { path: String },

    #[error("Invalid VPN server URL: {url}")]
    InvalidUrl { url: String },

    #[error("Missing required configuration field: {field}")]
    MissingField { field: String },

    #[error("Configuration validation error: {message}")]
    ValidationError { message: String },

    #[error("I/O error: {message}")]
    IoError { message: String },
}

/// GNOME Keyring operation errors
#[derive(Error, Debug)]
pub enum KeyringError {
    #[error("Keyring service unavailable")]
    ServiceUnavailable,

    #[error("Failed to store credential in keyring")]
    StoreFailed,

    #[error("Failed to retrieve credential from keyring")]
    RetrieveFailed,

    #[error("Credential not found in keyring")]
    NotFound,

    #[error("Keyring is locked")]
    Locked,

    #[error("Invalid credential format")]
    InvalidFormat,

    #[error("PIN not found in keyring")]
    PinNotFound,

    #[error("OTP secret not found in keyring")]
    OtpSecretNotFound,
}

/// VPN connection operation errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum VpnError {
    #[error("Connection failed: {reason}")]
    ConnectionFailed { reason: String },

    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Network error: {reason}")]
    NetworkError { reason: String },

    #[error("OpenConnect library error: {code}")]
    OpenConnectError { code: i32 },

    #[error("Invalid connection state transition")]
    InvalidStateTransition,

    #[error("Failed to spawn OpenConnect process: {reason}")]
    ProcessSpawnError { reason: String },

    #[error("Connection timeout after {seconds} seconds")]
    ConnectionTimeout { seconds: u64 },

    #[error("Failed to terminate OpenConnect process")]
    TerminationError,

    #[error("Failed to parse OpenConnect output: {line}")]
    ParseError { line: String },
}

/// OTP/TOTP operation errors
#[derive(Error, Debug, PartialEq)]
pub enum OtpError {
    #[error("Invalid Base32 secret")]
    InvalidBase32,

    #[error("TOTP generation failed")]
    GenerationFailed,

    #[error("Time synchronization issue")]
    TimeSyncError,

    #[error("System time error")]
    TimeError,

    #[error("Invalid PIN format: must be exactly 4 numeric digits")]
    InvalidPinFormat,

    #[error("HMAC-SHA1 computation failed")]
    HmacFailed,

    #[error("Invalid HOTP counter")]
    InvalidCounter,
}

/// Result type alias for convenience
pub type Result<T> = std::result::Result<T, AkonError>;
