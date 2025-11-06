//! Tests for reconnection configuration parsing

use akon_core::config::toml_config::TomlConfig;
use std::path::PathBuf;

#[test]
fn test_parse_reconnection_config_from_file() {
    // Given: A config file with reconnection settings
    let config_path = PathBuf::from("tests/fixtures/test_config.toml");

    // When: Loading the config
    let config = TomlConfig::from_file(&config_path).expect("Should load config");

    // Then: Should have reconnection policy with correct values
    let policy = config
        .reconnection_policy()
        .expect("Should have reconnection policy");
    assert_eq!(policy.max_attempts, 5);
    assert_eq!(policy.base_interval_secs, 5);
    assert_eq!(policy.backoff_multiplier, 2);
    assert_eq!(policy.max_interval_secs, 60);
    assert_eq!(policy.consecutive_failures_threshold, 3);
    assert_eq!(policy.health_check_interval_secs, 60);
    assert_eq!(
        policy.health_check_endpoint,
        "https://vpn.example.com/healthz"
    );
}

#[test]
fn test_parse_custom_reconnection_config() {
    // Given: A config file with custom reconnection settings
    let config_path = PathBuf::from("tests/fixtures/custom_config.toml");

    // When: Loading the config
    let config = TomlConfig::from_file(&config_path).expect("Should load config");

    // Then: Should have custom values
    let policy = config
        .reconnection_policy()
        .expect("Should have reconnection policy");
    assert_eq!(policy.max_attempts, 10);
    assert_eq!(policy.base_interval_secs, 10);
    assert_eq!(policy.backoff_multiplier, 3);
    assert_eq!(policy.max_interval_secs, 120);
    assert_eq!(policy.consecutive_failures_threshold, 5);
    assert_eq!(policy.health_check_interval_secs, 30);
    assert_eq!(
        policy.health_check_endpoint,
        "https://vpn.test.local/health"
    );
}

#[test]
fn test_reconnection_config_defaults_when_section_missing() {
    // Given: A minimal config without reconnection section
    let config_toml = r#"
        [vpn]
        server = "vpn.example.com"
        username = "testuser"
    "#;

    // When: Parsing the config
    let config: TomlConfig = toml::from_str(config_toml).expect("Should parse");

    // Then: Should return None when section is missing (use defaults at runtime instead)
    assert!(
        config.reconnection_policy().is_none(),
        "Should be None when section missing"
    );
}

#[test]
fn test_validate_max_attempts_range() {
    // Given: A policy with invalid max_attempts (too high)
    let config_toml = r#"
        [reconnection]
        max_attempts = 25
        health_check_endpoint = "https://vpn.example.com/health"
    "#;

    // When: Parsing the config
    let result: Result<TomlConfig, _> = toml::from_str(config_toml);

    // Then: Should either reject or clamp the value
    // (Implementation will decide - either validation error or clamping to 20)
    match result {
        Ok(config) => {
            let policy = config.reconnection_policy().unwrap();
            assert!(
                policy.max_attempts <= 20,
                "max_attempts should be clamped to 20"
            );
        }
        Err(_) => {
            // Validation error is also acceptable
        }
    }
}

#[test]
fn test_validate_backoff_multiplier_range() {
    // Given: A policy with invalid backoff_multiplier
    let config_toml = r#"
        [reconnection]
        backoff_multiplier = 15
        health_check_endpoint = "https://vpn.example.com/health"
    "#;

    // When: Parsing the config
    let result: Result<TomlConfig, _> = toml::from_str(config_toml);

    // Then: Should handle invalid multiplier
    match result {
        Ok(config) => {
            let policy = config.reconnection_policy().unwrap();
            assert!(
                policy.backoff_multiplier <= 10,
                "backoff_multiplier should be clamped"
            );
        }
        Err(_) => {
            // Validation error is acceptable
        }
    }
}

#[test]
fn test_health_check_endpoint_required() {
    // Given: A config with reconnection but missing endpoint
    let config_toml = r#"
        [reconnection]
        max_attempts = 5
    "#;

    // When: Parsing the config
    let result: Result<TomlConfig, _> = toml::from_str(config_toml);

    // Then: Should fail because endpoint is required
    assert!(result.is_err(), "Should require health_check_endpoint");
}
