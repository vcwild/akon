//! Integration tests for get-password command
//!
//! Tests the get-password command behavior including error handling
//! and output format validation.

use std::process::Command;

#[test]
fn test_get_password_command_exists() {
    let output = Command::new("cargo")
        .args(&["run", "--quiet", "--", "get-password", "--help"])
        .output()
        .expect("Failed to run get-password --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("get-password"));
}

#[test]
fn test_get_password_requires_setup() {
    // This test assumes no prior setup has been done
    // In a real environment, this would fail with exit code 2 (config error)
    let output = Command::new("cargo")
        .args(&["run", "--quiet", "--", "get-password"])
        .output()
        .expect("Failed to run get-password without setup");

    // Should exit with code 2 (config error - no config file)
    assert_eq!(output.status.code(), Some(2));
}
