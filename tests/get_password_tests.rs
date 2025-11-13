//! Integration tests for get-password command
//!
//! Tests the get-password command behavior including error handling
//! and output format validation.

use akon_core::types::{KEYRING_SERVICE_OTP, KEYRING_SERVICE_PIN};
use std::io::Write;
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
    // Skip this test in CI environments as it requires system keyring access
    // The mock keyring is used for unit tests, but integration tests with
    // the binary still need the real keyring
    if std::env::var("CI").is_ok() {
        eprintln!("Skipping test_get_password_with_temp_config in CI environment");
        return;
    }

    // Create a temporary directory for config
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let temp_config_dir = temp_dir.path().to_string_lossy().to_string();

    // Set environment variable for config directory
    env::set_var("AKON_CONFIG_DIR", &temp_config_dir);

    // Create test config
    let config_content = r#"
server = "test.vpn.example.com"
username = "__akon_get_password_test__"
timeout = 30
"#;
    fs::create_dir_all(&temp_config_dir).expect("Failed to create config dir");
    fs::write(
        std::path::Path::new(&temp_config_dir).join("config.toml"),
        config_content,
    )
    .expect("Failed to write config file");

    // Store test credentials in the system keyring so the spawned binary can read them.
    // We do this with `secret-tool` (GNOME keyring). This test is intended for
    // local interactive environments and is skipped in CI above.
    let test_username = "__akon_get_password_test__";
    let test_secret = "JBSWY3DPEHPK3PXP"; // Valid base32
    let test_pin_value = "1234";

    // Helper to store a secret using `secret-tool store` by writing the secret to stdin.
    fn store_system_secret(service: &str, username: &str, secret: &str) -> Result<(), String> {
        let mut child = Command::new("secret-tool")
            .args(["store", "--label", "akon-test", "service", service, "username", username])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn secret-tool: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(secret.as_bytes())
                .map_err(|e| format!("failed writing to secret-tool stdin: {}", e))?;
        }

        let status = child
            .wait()
            .map_err(|e| format!("failed waiting for secret-tool: {}", e))?;

        if status.success() {
            Ok(())
        } else {
            Err(format!("secret-tool exited with status: {:?}", status.code()))
        }
    }

    store_system_secret(KEYRING_SERVICE_OTP, test_username, test_secret)
        .expect("Failed to store test OTP secret in system keyring");

    store_system_secret(KEYRING_SERVICE_PIN, test_username, test_pin_value)
        .expect("Failed to store test PIN in system keyring");

    // Run get-password command
    let output = Command::new(AKON_BINARY)
        .args(["get-password"])
        .env("AKON_CONFIG_DIR", &temp_config_dir)
        .output()
        .expect("Failed to run get-password");

    // Clean up: remove stored secrets from system keyring and restore env
    let _ = Command::new("secret-tool")
        .args(["clear", "service", KEYRING_SERVICE_OTP, "username", test_username])
        .status();
    let _ = Command::new("secret-tool")
        .args(["clear", "service", KEYRING_SERVICE_PIN, "username", test_username])
        .status();
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
