//! Integration tests for Error state handling and status command output
//!
//! These tests verify that when the reconnection manager enters Error state
//! after exhausting all retry attempts, the status command provides clear
//! suggestions for manual intervention.

use std::fs;
use std::process::Command;

/// Helper to create a test state file
fn create_state_file(name: &str, content: &str) {
    fs::write(format!("/tmp/{}.json", name), content).expect("Failed to write test state file");
}

/// Helper to cleanup test state file
fn cleanup_state_file(name: &str) {
    let _ = fs::remove_file(format!("/tmp/{}.json", name));
}

#[test]
#[ignore] // Requires serial execution
fn test_status_command_suggests_reset_on_error_state() {
    // Create Error state file
    create_state_file(
        "akon_vpn_state_error",
        r#"{
  "state": "Error",
  "error": "Max reconnection attempts (5) exceeded",
  "max_attempts": 5,
  "updated_at": "2025-11-05T12:00:00Z"
}"#,
    );

    // Run status command
    let output = Command::new("cargo")
        .args(["run", "--", "vpn", "status"])
        .env("AKON_STATE_FILE", "/tmp/akon_vpn_state_error.json")
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Verify exit code is 3 (error state)
    assert_eq!(
        output.status.code(),
        Some(3),
        "Expected exit code 3 for Error state"
    );

    // Verify Error state is detected
    assert!(
        combined.contains("Status: Error"),
        "Should show Error status"
    );

    // Verify error message is shown
    assert!(
        combined.contains("Max reconnection attempts"),
        "Should show max attempts exceeded message"
    );

    // Verify manual intervention suggestions are provided
    assert!(
        combined.contains("Manual intervention required"),
        "Should suggest manual intervention"
    );

    assert!(
        combined.contains("akon vpn off"),
        "Should suggest disconnect command"
    );

    assert!(
        combined.contains("akon vpn on --force"),
        "Should suggest reconnect command"
    );

    assert!(
        combined.contains("akon vpn on --force"),
        "Should suggest reconnect command"
    );

    // Cleanup
    cleanup_state_file("akon_vpn_state_error");

    println!("✓ Test passed: Status command provides helpful suggestions for Error state");
}

#[test]
#[ignore] // Requires release binary and serial execution
fn test_status_command_shows_reconnecting_state() {
    // Create Reconnecting state file
    create_state_file(
        "akon_vpn_state_reconnect",
        r#"{
  "state": "Reconnecting",
  "attempt": 2,
  "next_retry_at": 1730815200,
  "max_attempts": 5,
  "updated_at": "2025-11-05T12:00:00Z"
}"#,
    );

    // Run status command
    let output = Command::new("cargo")
        .args(["run", "--", "vpn", "status"])
        .env("AKON_STATE_FILE", "/tmp/akon_vpn_state_reconnect.json")
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Verify exit code is 1 (reconnecting is considered disconnected)
    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for reconnecting state"
    );

    // Verify Reconnecting state is shown
    assert!(
        combined.contains("Status: Reconnecting") || combined.contains("Reconnecting"),
        "Should show Reconnecting status"
    );

    assert!(
        combined.contains("Attempt 2 of 5"),
        "Should show current attempt"
    );

    // Cleanup
    cleanup_state_file("akon_vpn_state_reconnect");

    println!("✓ Test passed: Status command shows Reconnecting state details");
}

#[test]
#[ignore] // Requires release binary and serial execution
fn test_status_command_shows_disconnected_when_no_state_file() {
    // Ensure no state file exists
    cleanup_state_file("akon_vpn_state_disconnect");

    // Run status command
    let output = Command::new("cargo")
        .args(["run", "--", "vpn", "status"])
        .env("AKON_STATE_FILE", "/tmp/akon_vpn_state_disconnect.json")
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Verify exit code is 1 (not connected)
    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit code 1 for not connected"
    );

    // Verify disconnected message
    assert!(
        combined.contains("Not connected"),
        "Should show Not connected status"
    );

    println!("✓ Test passed: Status command shows Not connected when no state file");
}

#[test]
#[ignore] // Requires release binary and serial execution
fn test_error_state_shows_attempt_count() {
    // Create Error state file with attempt information
    create_state_file(
        "akon_vpn_state_attempt",
        r#"{
  "state": "Error",
  "error": "Max reconnection attempts (3) exceeded",
  "max_attempts": 3,
  "updated_at": "2025-11-05T12:00:00Z"
}"#,
    );

    // Run status command
    let output = Command::new("cargo")
        .args(["run", "--", "vpn", "status"])
        .env("AKON_STATE_FILE", "/tmp/akon_vpn_state_attempt.json")
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Verify attempt count is shown
    assert!(
        combined.contains("Failed after 3 reconnection attempts"),
        "Should show number of failed attempts"
    );

    // Cleanup
    cleanup_state_file("akon_vpn_state_attempt");

    println!("✓ Test passed: Error state shows attempt count");
}
