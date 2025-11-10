//! Integration tests for reconnection configuration
//!
//! Tests that configuration values are properly loaded and applied
//! to reconnection behavior.

use akon_core::config::toml_config::{self, TomlConfig};
use akon_core::config::VpnConfig;
use akon_core::vpn::reconnection::ReconnectionPolicy;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a temporary config file
fn create_test_config_file(
    dir: &TempDir,
    filename: &str,
    vpn_config: &VpnConfig,
    reconnection_policy: Option<&ReconnectionPolicy>,
) -> PathBuf {
    let path = dir.path().join(filename);
    toml_config::save_complete_config_to_path(vpn_config, reconnection_policy, &path)
        .expect("Failed to save test config");
    path
}

/// Helper to create a basic VPN config for testing
fn create_test_vpn_config() -> VpnConfig {
    VpnConfig {
        server: "vpn.example.com".to_string(),
        username: "testuser".to_string(),
        protocol: Default::default(),
        timeout: Some(30),
        no_dtls: false,
        lazy_mode: false,
    }
}

#[test]
fn test_config_with_default_reconnection_policy() {
    // Create config with defaults
    let vpn_config = create_test_vpn_config();
    let reconnection_policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://www.google.com".to_string(),
    };

    // Save and load
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config_file(
        &temp_dir,
        "default.toml",
        &vpn_config,
        Some(&reconnection_policy),
    );

    let loaded = TomlConfig::from_file(&config_path).expect("Failed to load config");

    // Verify VPN config
    assert_eq!(loaded.vpn_config.server, "vpn.example.com");
    assert_eq!(loaded.vpn_config.username, "testuser");

    // Verify reconnection policy
    let policy = loaded
        .reconnection
        .expect("Reconnection policy should be present");
    assert_eq!(policy.max_attempts, 5);
    assert_eq!(policy.base_interval_secs, 5);
    assert_eq!(policy.backoff_multiplier, 2);
    assert_eq!(policy.max_interval_secs, 60);
    assert_eq!(policy.consecutive_failures_threshold, 3);
    assert_eq!(policy.health_check_interval_secs, 60);
    assert_eq!(policy.health_check_endpoint, "https://www.google.com");
}

#[test]
fn test_config_with_custom_reconnection_policy() {
    // Create config with custom values
    let vpn_config = create_test_vpn_config();
    let reconnection_policy = ReconnectionPolicy {
        max_attempts: 10,
        base_interval_secs: 10,
        backoff_multiplier: 3,
        max_interval_secs: 120,
        consecutive_failures_threshold: 5,
        health_check_interval_secs: 30,
        health_check_endpoint: "https://vpn-gateway.example.com/health".to_string(),
    };

    // Save and load
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config_file(
        &temp_dir,
        "custom.toml",
        &vpn_config,
        Some(&reconnection_policy),
    );

    let loaded = TomlConfig::from_file(&config_path).expect("Failed to load config");

    // Verify custom values
    let policy = loaded
        .reconnection
        .expect("Reconnection policy should be present");
    assert_eq!(policy.max_attempts, 10);
    assert_eq!(policy.base_interval_secs, 10);
    assert_eq!(policy.backoff_multiplier, 3);
    assert_eq!(policy.max_interval_secs, 120);
    assert_eq!(policy.consecutive_failures_threshold, 5);
    assert_eq!(policy.health_check_interval_secs, 30);
    assert_eq!(
        policy.health_check_endpoint,
        "https://vpn-gateway.example.com/health"
    );
}

#[test]
fn test_config_without_reconnection_policy() {
    // Create config without reconnection section
    let vpn_config = create_test_vpn_config();

    // Save and load
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config_file(&temp_dir, "no_reconnection.toml", &vpn_config, None);

    let loaded = TomlConfig::from_file(&config_path).expect("Failed to load config");

    // Verify VPN config
    assert_eq!(loaded.vpn_config.server, "vpn.example.com");

    // Verify no reconnection policy
    assert!(
        loaded.reconnection.is_none(),
        "Reconnection policy should be None"
    );
}

#[test]
fn test_config_validation_rejects_invalid_max_attempts() {
    let vpn_config = create_test_vpn_config();
    let invalid_policy = ReconnectionPolicy {
        max_attempts: 0, // Invalid: must be >= 1
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://www.google.com".to_string(),
    };

    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("invalid.toml");

    // Should fail validation
    let result =
        toml_config::save_complete_config_to_path(&vpn_config, Some(&invalid_policy), &path);
    assert!(result.is_err(), "Should reject invalid max_attempts");
}

#[test]
fn test_config_validation_rejects_invalid_base_interval() {
    let vpn_config = create_test_vpn_config();
    let invalid_policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 0, // Invalid: must be >= 1
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://www.google.com".to_string(),
    };

    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("invalid.toml");

    // Should fail validation
    let result =
        toml_config::save_complete_config_to_path(&vpn_config, Some(&invalid_policy), &path);
    assert!(result.is_err(), "Should reject invalid base_interval");
}

#[test]
fn test_config_validation_rejects_invalid_endpoint() {
    let vpn_config = create_test_vpn_config();
    let invalid_policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "not-a-valid-url".to_string(), // Invalid: not HTTP/HTTPS
    };

    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("invalid.toml");

    // Should fail validation
    let result =
        toml_config::save_complete_config_to_path(&vpn_config, Some(&invalid_policy), &path);
    assert!(
        result.is_err(),
        "Should reject invalid health_check_endpoint"
    );
}

#[test]
fn test_backoff_calculation_respects_config() {
    use akon_core::vpn::reconnection::ReconnectionManager;

    // Create policy with specific backoff parameters
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 10, // Base: 10s
        backoff_multiplier: 3,  // Multiplier: 3x
        max_interval_secs: 200,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://www.google.com".to_string(),
    };

    // Create reconnection manager
    let manager = ReconnectionManager::new(policy.clone());

    // Test backoff calculation
    // Attempt 1: 10 * 3^0 = 10s
    let backoff1 = manager.calculate_backoff(1);
    assert_eq!(backoff1.as_secs(), 10);

    // Attempt 2: 10 * 3^1 = 30s
    let backoff2 = manager.calculate_backoff(2);
    assert_eq!(backoff2.as_secs(), 30);

    // Attempt 3: 10 * 3^2 = 90s
    let backoff3 = manager.calculate_backoff(3);
    assert_eq!(backoff3.as_secs(), 90);

    // Attempt 4: 10 * 3^3 = 270s, but capped at max_interval (200s)
    let backoff4 = manager.calculate_backoff(4);
    assert_eq!(backoff4.as_secs(), 200);
}

#[test]
fn test_config_roundtrip_preserves_all_values() {
    // Create config with all fields set
    let vpn_config = VpnConfig {
        server: "vpn.test.com".to_string(),
        username: "testuser123".to_string(),
        protocol: Default::default(),
        timeout: Some(45),
        no_dtls: true,
        lazy_mode: true,
    };

    let reconnection_policy = ReconnectionPolicy {
        max_attempts: 7,
        base_interval_secs: 15,
        backoff_multiplier: 4,
        max_interval_secs: 180,
        consecutive_failures_threshold: 4,
        health_check_interval_secs: 45,
        health_check_endpoint: "https://health.example.com/check".to_string(),
    };

    // Save and load
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config_file(
        &temp_dir,
        "roundtrip.toml",
        &vpn_config,
        Some(&reconnection_policy),
    );

    let loaded = TomlConfig::from_file(&config_path).expect("Failed to load config");

    // Verify VPN config roundtrip
    assert_eq!(loaded.vpn_config.server, "vpn.test.com");
    assert_eq!(loaded.vpn_config.username, "testuser123");
    assert_eq!(loaded.vpn_config.timeout, Some(45));
    assert!(loaded.vpn_config.no_dtls);
    assert!(loaded.vpn_config.lazy_mode);

    // Verify reconnection policy roundtrip
    let policy = loaded
        .reconnection
        .expect("Reconnection policy should be present");
    assert_eq!(policy.max_attempts, 7);
    assert_eq!(policy.base_interval_secs, 15);
    assert_eq!(policy.backoff_multiplier, 4);
    assert_eq!(policy.max_interval_secs, 180);
    assert_eq!(policy.consecutive_failures_threshold, 4);
    assert_eq!(policy.health_check_interval_secs, 45);
    assert_eq!(
        policy.health_check_endpoint,
        "https://health.example.com/check"
    );
}

/// Note: This test documents what would need to be tested with a live VPN connection
///
/// To fully validate T040 with a real VPN connection, the following scenarios should be tested:
///
/// 1. **Verify max_attempts behavior**:
///    - Set max_attempts to 3
///    - Trigger network interruption
///    - Verify exactly 3 reconnection attempts occur before giving up
///
/// 2. **Verify backoff intervals**:
///    - Set base_interval_secs to 5, backoff_multiplier to 2
///    - Trigger multiple reconnection attempts
///    - Measure actual wait times between attempts (should be 5s, 10s, 20s, ...)
///
/// 3. **Verify health check behavior**:
///    - Set health_check_interval_secs to 30
///    - Set consecutive_failures_threshold to 2
///    - Monitor health check timing (should run every 30s)
///    - Block health endpoint and verify reconnection after 2 failures
///
/// 4. **Verify endpoint configuration**:
///    - Set health_check_endpoint to custom URL
///    - Monitor network requests to verify correct endpoint is used
///
/// These tests require:
/// - Live VPN connection with credentials
/// - Ability to simulate network interruptions
/// - Ability to block specific endpoints
/// - Time measurement infrastructure
///
/// Run with: cargo test --test config_integration_tests -- --ignored --test-threads=1
#[test]
#[ignore = "Requires live VPN connection"]
fn test_live_vpn_reconnection_with_config() {
    // This test would:
    // 1. Load config with custom reconnection settings
    // 2. Connect to VPN
    // 3. Trigger network interruption
    // 4. Verify reconnection attempts match config
    // 5. Measure actual backoff timing
    // 6. Verify health checks use configured endpoint and interval

    todo!("Implement with live VPN setup");
}
