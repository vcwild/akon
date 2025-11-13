//! Unit tests for VPN configuration validation
//!
//! Tests VpnConfig validation logic to ensure proper input validation.

use akon_core::config::VpnConfig;

#[test]
fn test_valid_config() {
    let config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn test_empty_server() {
    let config = VpnConfig::new("".to_string(), "testuser".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Server cannot be empty");
}

#[test]
fn test_invalid_server_characters() {
    let config = VpnConfig::new("server!".to_string(), "testuser".to_string());
    assert!(config.validate().is_err());
    assert_eq!(
        config.validate().unwrap_err(),
        "Server contains invalid characters"
    );
}

#[test]
fn test_empty_username() {
    let config = VpnConfig::new("vpn.example.com".to_string(), "".to_string());
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Username cannot be empty");
}

#[test]
fn test_zero_timeout() {
    let mut config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());
    config.timeout = Some(0);
    assert!(config.validate().is_err());
    assert_eq!(config.validate().unwrap_err(), "Timeout cannot be zero");
}

#[test]
fn test_valid_config_with_optional_fields() {
    let mut config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());
    config.timeout = Some(60);
    assert!(config.validate().is_ok());
}

#[test]
fn test_server_with_dashes_and_dots() {
    let config = VpnConfig::new("vpn-server.example.com".to_string(), "testuser".to_string());
    assert!(config.validate().is_ok());
}

#[test]
fn test_server_with_numbers() {
    let config = VpnConfig::new("vpn123.example.com".to_string(), "testuser".to_string());
    assert!(config.validate().is_ok());
}

// ===== ReconnectionPolicy Tests (T039) =====

mod reconnection_policy_tests {
    use akon_core::vpn::reconnection::ReconnectionPolicy;

    #[test]
    fn test_parse_reconnection_config_with_all_fields() {
        // Test parsing a complete config with all fields specified
        let toml_str = r#"
            max_attempts = 10
            base_interval_secs = 10
            backoff_multiplier = 3
            max_interval_secs = 120
            consecutive_failures_threshold = 5
            health_check_interval_secs = 90
            health_check_endpoint = "https://vpn.example.com/health"
        "#;

        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();

        assert_eq!(policy.max_attempts, 10);
        assert_eq!(policy.base_interval_secs, 10);
        assert_eq!(policy.backoff_multiplier, 3);
        assert_eq!(policy.max_interval_secs, 120);
        assert_eq!(policy.consecutive_failures_threshold, 5);
        assert_eq!(policy.health_check_interval_secs, 90);
        assert_eq!(
            policy.health_check_endpoint,
            "https://vpn.example.com/health"
        );
    }

    #[test]
    fn test_parse_reconnection_config_with_defaults() {
        // Test parsing with only required field (endpoint), rest should use defaults
        let toml_str = r#"
            health_check_endpoint = "https://vpn.example.com/health"
        "#;

        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();

        // Check defaults are applied
        assert_eq!(policy.max_attempts, 3); // default (updated)
        assert_eq!(policy.base_interval_secs, 5); // default
        assert_eq!(policy.backoff_multiplier, 2); // default
        assert_eq!(policy.max_interval_secs, 60); // default
        assert_eq!(policy.consecutive_failures_threshold, 1); // default (updated)
        assert_eq!(policy.health_check_interval_secs, 10); // default (updated)
        assert_eq!(
            policy.health_check_endpoint,
            "https://vpn.example.com/health"
        );
    }

    #[test]
    fn test_validate_max_attempts_range() {
        // max_attempts must be 1-20
        let toml_str = r#"
            max_attempts = 0
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy
            .validate()
            .unwrap_err()
            .to_string()
            .contains("max_attempts"));

        let toml_str = r#"
            max_attempts = 21
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy
            .validate()
            .unwrap_err()
            .to_string()
            .contains("max_attempts"));

        let toml_str = r#"
            max_attempts = 10
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_backoff_multiplier_range() {
        // backoff_multiplier must be 1-10
        let toml_str = r#"
            backoff_multiplier = 0
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy
            .validate()
            .unwrap_err()
            .to_string()
            .contains("backoff_multiplier"));

        let toml_str = r#"
            backoff_multiplier = 11
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy
            .validate()
            .unwrap_err()
            .to_string()
            .contains("backoff_multiplier"));

        let toml_str = r#"
            backoff_multiplier = 5
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_health_check_endpoint_url() {
        // endpoint must be valid HTTP/HTTPS URL
        let toml_str = r#"
            health_check_endpoint = "ftp://invalid.com"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy.validate().unwrap_err().to_string().contains("http"));

        let toml_str = r#"
            health_check_endpoint = "not a url"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_err());
        assert!(policy.validate().unwrap_err().to_string().contains("parse"));

        let toml_str = r#"
            health_check_endpoint = "https://vpn.example.com/health"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_invalid_config_returns_error() {
        // Test that invalid configs return descriptive errors
        let toml_str = r#"
            max_attempts = 100
            base_interval_secs = 0
            consecutive_failures_threshold = 20
            health_check_endpoint = "invalid"
        "#;
        let policy: ReconnectionPolicy = toml::from_str(toml_str).unwrap();
        let validation_result = policy.validate();
        assert!(validation_result.is_err());
        let err = validation_result.unwrap_err().to_string();
        // Should fail on max_attempts first (validation order)
        assert!(err.contains("max_attempts"));
    }
}
