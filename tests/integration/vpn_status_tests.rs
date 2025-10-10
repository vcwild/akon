//! Integration tests for vpn status command
//!
//! Tests the vpn status command behavior including daemon communication
//! and state reporting.

use std::process::Command;

const AKON_BINARY: &str = "target/debug/akon";

#[test]
fn test_vpn_status_command_exists() {
    let output = Command::new(AKON_BINARY)
        .args(&["vpn", "status", "--help"])
        .output()
        .expect("Failed to run vpn status --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status"));
}

#[test]
fn test_vpn_status_no_daemon() {
    // Test status when no daemon is running
    let output = Command::new(AKON_BINARY)
        .args(&["vpn", "status"])
        .output()
        .expect("Failed to run vpn status without daemon");

    // Should succeed and show disconnected status
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("disconnected") || stdout.contains("not connected"));
}
