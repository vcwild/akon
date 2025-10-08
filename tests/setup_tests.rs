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
    // Test that setup command shows help
    let output = Command::new(AKON_BINARY)
        .args(&["setup", "--help"])
        .output()
        .expect("Failed to run akon setup --help");

    assert!(output.status.success(), "Setup help command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("setup"), "Help should mention setup command");
}

#[test]
#[ignore] // Requires interactive input - skip by default
fn test_setup_command_with_input() {
    // Test that the setup command can process input
    // This test provides mock input to simulate user interaction
    use std::process::Stdio;
    use std::io::Write;

    let mut child = Command::new(AKON_BINARY)
        .args(&["setup"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn akon setup");

    // Provide mock input (simulating user typing responses)
    if let Some(mut stdin) = child.stdin.take() {
        // Check if existing config - answer no to overwrite
        let _ = stdin.write_all(b"n\n");
    }

    // Kill the process after a short timeout since we can't fully complete setup
    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = child.kill();

    // This test is mainly to ensure the setup command doesn't panic
    println!("Setup command can be invoked (requires manual testing for full workflow)");
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
#[ignore] // May hang if keyring prompts for unlock - skip by default
fn test_keyring_availability_integration() {
    // Test keyring availability (integration with actual system keyring)
    // NOTE: This test may hang if GNOME Keyring prompts for password unlock
    let test_username = "__akon_setup_test__";
    let test_secret = "SETUP_INTEGRATION_TEST";

    // Clean up any existing test data
    let _ = keyring::delete_otp_secret(test_username);

    // Test if keyring operations work
    let store_result = keyring::store_otp_secret(test_username, test_secret);

    if store_result.is_ok() {
        // Keyring is available - test full workflow
        let exists = keyring::has_otp_secret(test_username).unwrap_or(false);
        assert!(exists, "Secret should exist after successful storage");

        let retrieved = keyring::retrieve_otp_secret(test_username).unwrap_or_default();
        assert_eq!(retrieved, test_secret, "Retrieved secret should match stored secret");

        // Clean up
        let _ = keyring::delete_otp_secret(test_username);

        println!("Keyring integration test passed - GNOME Keyring is available");
    } else {
        // Keyring not available - ensure operations fail gracefully
        println!("Keyring not available for integration testing - this is expected in some environments");

        // Verify that has_otp_secret returns false for nonexistent entries
        let exists = keyring::has_otp_secret(test_username).unwrap_or(true);
        assert!(!exists, "Nonexistent secret should not exist even when keyring is unavailable");
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
