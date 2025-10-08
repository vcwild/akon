//! Unit tests for VPN configuration validation
//!
//! Tests VpnConfig validation logic to ensure proper input validation.

use akon_core::config::VpnConfig;

#[test]
fn test_valid_config() {
    let config = VpnConfig::new("vpn.example.com".to_string(), 443, "testuser".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn test_empty_server() {
    let config = VpnConfig::new("".to_string(), 443, "testuser".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Server cannot be empty");
}

#[test]
fn test_invalid_server_characters() {
    let config = VpnConfig::new("server!".to_string(), 443, "testuser".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Server contains invalid characters");
}

#[test]
fn test_zero_port() {
    let config = VpnConfig::new("vpn.example.com".to_string(), 0, "testuser".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Port cannot be zero");
}

#[test]
fn test_empty_username() {
    let config = VpnConfig::new("vpn.example.com".to_string(), 443, "".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Username cannot be empty");
}

#[test]
fn test_zero_timeout() {
    let mut config = VpnConfig::new("vpn.example.com".to_string(), 443, "testuser".to_string());
    config.timeout = Some(0);
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Timeout cannot be zero");
}

#[test]
fn test_valid_config_with_optional_fields() {
    let mut config = VpnConfig::new("vpn.example.com".to_string(), 4443, "testuser".to_string());
    config.realm = Some("realm1".to_string());
    config.timeout = Some(60);
    assert!(config.validate().is_ok());
}

#[test]
fn test_server_with_dashes_and_dots() {
    let config = VpnConfig::new("vpn-server.example.com".to_string(), 443, "testuser".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn test_server_with_numbers() {
    let config = VpnConfig::new("vpn123.example.com".to_string(), 443, "testuser".to_string());
    assert!(config.validate().is_ok());
}
