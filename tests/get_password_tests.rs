//! Integration tests for get-password command
//!
//! Tests the get-password command behavior including error handling
//! and output format validation.

use akon_core::auth::keyring;
use std::env;
use std::fs;
use std::process::Command;

const AKON_BINARY: &str = "target/debug/akon";

#[test]
fn test_get_password_command_exists() {
    let output = Command::new(AKON_BINARY)
        .args(["get-password", "--help"])
        .output()
        .expect("Failed to run get-password --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("get-password"));
}

#[test]
fn test_get_password_with_temp_config() {
    // Create a temporary directory for config
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let temp_config_dir = temp_dir.path().to_string_lossy().to_string();

    // Set environment variable for config directory
    env::set_var("AKON_CONFIG_DIR", &temp_config_dir);

    // Create test config
    let config_content = r#"
server = "test.vpn.example.com"
port = 443
username = "__akon_get_password_test__"
timeout = 30
"#;
    fs::create_dir_all(&temp_config_dir).expect("Failed to create config dir");
    fs::write(
        std::path::Path::new(&temp_config_dir).join("config.toml"),
        config_content,
    )
    .expect("Failed to write config file");

    // Store test credentials in keyring
    let test_username = "__akon_get_password_test__";
    let test_secret = "JBSWY3DPEHPK3PXP"; // Valid base32
    let test_pin = akon_core::types::Pin::new("1234".to_string()).expect("Valid PIN");

    keyring::store_otp_secret(test_username, test_secret).expect("Failed to store test OTP secret");
    keyring::store_pin(test_username, &test_pin).expect("Failed to store test PIN");

    // Run get-password command
    let output = Command::new(AKON_BINARY)
        .args(["get-password"])
        .env("AKON_CONFIG_DIR", &temp_config_dir)
        .output()
        .expect("Failed to run get-password");

    // Clean up
    let _ = keyring::delete_otp_secret(test_username);
    let _ = keyring::delete_pin(test_username);
    env::remove_var("AKON_CONFIG_DIR");

    // Should succeed and output a complete password (PIN + OTP)
    assert!(
        output.status.success(),
        "get-password should succeed with valid config and keyring"
    );
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should output exactly 10 characters (4-digit PIN + 6-digit OTP)
    assert_eq!(
        stdout.len(),
        10,
        "Password should be exactly 10 characters (PIN + OTP), got: '{}' with length {}",
        stdout,
        stdout.len()
    );
    assert!(
        stdout.chars().all(|c| c.is_numeric()),
        "Password should contain only digits, got: '{}'",
        stdout
    );

    // Verify it starts with the PIN
    assert!(
        stdout.starts_with("1234"),
        "Password should start with PIN '1234', got: '{}'",
        stdout
    );

    // Should have no stderr output for success case
    assert!(
        stderr.is_empty(),
        "Should have no stderr output on success, got: '{}'",
        stderr
    );
}

#[test]
fn test_get_password_missing_config() {
    // Create a temporary directory for config (but don't create config file)
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let temp_config_dir = temp_dir.path().to_string_lossy().to_string();

    // Set environment variable for config directory
    env::set_var("AKON_CONFIG_DIR", &temp_config_dir);

    // Ensure no config file exists
    let config_path = std::path::Path::new(&temp_config_dir).join("config.toml");
    if config_path.exists() {
        fs::remove_file(&config_path).expect("Failed to remove config file");
    }

    // Run get-password command
    let output = Command::new(AKON_BINARY)
        .args(["get-password"])
        .env("AKON_CONFIG_DIR", &temp_config_dir)
        .output()
        .expect("Failed to run get-password without config");

    // Clean up
    env::remove_var("AKON_CONFIG_DIR");

    // Should exit with code 2 (config error)
    assert_eq!(
        output.status.code(),
        Some(2),
        "Should exit with code 2 for missing config"
    );

    // Should have error message on stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "Should have error message on stderr");
    assert!(
        stderr.contains("config") || stderr.contains("load"),
        "Error should mention config loading"
    );

    // Should have no stdout output for error case
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "Should have no stdout output on config error"
    );
}
