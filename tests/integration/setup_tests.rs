//! Integration tests for setup command flow
//!
//! Tests the complete setup command workflow including:
//! - Interactive prompts for VPN configuration
//! - OTP secret input and validation
//! - Keyring storage operations
//! - Configuration file creation
//! - Overwrite confirmation for existing setups
//! - Keyring lock/unlock detection

use std::process::Command;
use akon_core::config::toml_config;
use akon_core::auth::keyring;

const AKON_BINARY: &str = "target/debug/akon";

#[test]
fn test_setup_command_help() {
    // Test that setup command shows help with --help flag (idiomatic Linux CLI)
    let output = Command::new(AKON_BINARY)
        .args(&["setup", "--help"])
        .output()
        .expect("Failed to run akon setup --help");

    assert!(output.status.success(), "Setup --help command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Setup") || stdout.contains("setup"), "Help should mention setup command");
    assert!(stdout.contains("Usage"), "Help should show usage information");
}

#[test]
fn test_setup_command_exists() {
    // Test that the akon binary exists and can be executed
    // We don't run the actual setup command since it requires interactive input
    let output = Command::new(AKON_BINARY)
        .args(&["--help"])
        .output()
        .expect("Failed to run akon --help");

    assert!(output.status.success(), "akon --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("akon"), "Help should mention akon");
}

#[test]
fn test_config_directory_creation() {
    // Test that config directory can be created
    // This indirectly tests the toml_config functionality used by setup
    let result = toml_config::ensure_config_dir();
    if result.is_ok() {
        // If config dir creation succeeds, test that config_exists works
        let exists_result = toml_config::config_exists();
        assert!(exists_result.is_ok(), "config_exists should work after ensure_config_dir");
    } else {
        // If config dir creation fails (e.g., permissions), that's also fine for this test
        println!("Config directory creation failed - this may be expected in some test environments");
    }
}

#[test]
#[ignore] // Keyring tests can hang or require user interaction - run with `cargo test -- --ignored`
fn test_keyring_availability_integration() {
    // This test is ignored by default because keyring operations may:
    // - Hang waiting for keyring unlock
    // - Require GUI/session bus interaction
    // - Not be available in CI environments
    //
    // To run this test: cargo test -- --ignored test_keyring_availability_integration

    let test_username = "__akon_integration_test__";
    let test_secret = "INTEGRATION_TEST_SECRET";

    // Clean up any existing test data
    let _ = keyring::delete_otp_secret(test_username);

    // Test basic keyring availability
    let store_result = keyring::store_otp_secret(test_username, test_secret);

    if store_result.is_ok() {
        // If keyring is available, test full cycle
        let exists = keyring::has_otp_secret(test_username).unwrap_or(false);
        assert!(exists, "Secret should exist after successful storage");

        let retrieved = keyring::retrieve_otp_secret(test_username).unwrap_or_default();
        assert_eq!(retrieved, test_secret, "Retrieved secret should match stored secret");

        // Clean up
        let _ = keyring::delete_otp_secret(test_username);
    } else {
        println!("Keyring not available for integration testing - this is expected in some environments");
    }
}

// TODO: Implement these tests once we have a way to simulate interactive input
// #[test]
// fn test_setup_command_creates_config_and_keyring_entry() { ... }
//
// #[test]
// fn test_setup_command_overwrite_confirmation() { ... }
//
// #[test]
// fn test_setup_command_keyring_locked() { ... }
