//! Unit tests for error types and conversions

use akon_core::error::{AkonError, ConfigError, KeyringError, OtpError, VpnError};

#[test]
fn test_config_error_display() {
    let error = ConfigError::InvalidUrl {
        url: "invalid-url".to_string(),
    };
    assert_eq!(error.to_string(), "Invalid VPN server URL: invalid-url");
}

#[test]
fn test_keyring_error_display() {
    let error = KeyringError::NotFound;
    assert_eq!(error.to_string(), "Credential not found in keyring");
}

#[test]
fn test_vpn_error_display() {
    let error = VpnError::ConnectionFailed {
        reason: "timeout".to_string(),
    };
    assert_eq!(error.to_string(), "Connection failed: timeout");
}

#[test]
fn test_otp_error_display() {
    let error = OtpError::InvalidBase32;
    assert_eq!(error.to_string(), "Invalid Base32 secret");
}

#[test]
fn test_akon_error_from_config() {
    let config_error = ConfigError::MissingField {
        field: "username".to_string(),
    };
    let akon_error: AkonError = config_error.into();
    assert!(matches!(akon_error, AkonError::Config(_)));
}

#[test]
fn test_akon_error_from_io() {
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let akon_error: AkonError = io_error.into();
    assert!(matches!(akon_error, AkonError::Io(_)));
}

#[test]
fn test_akon_error_from_toml() {
    // Create a toml error by parsing invalid TOML
    let toml_error: toml::de::Error =
        toml::from_str::<serde_json::Value>("invalid toml").unwrap_err();
    let akon_error: AkonError = toml_error.into();
    assert!(matches!(akon_error, AkonError::Toml(_)));
}
