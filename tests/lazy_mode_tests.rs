//! Integration tests for lazy mode behavior
//!
//! These tests verify that running `akon` without arguments respects the
//! `lazy_mode` flag from the user configuration as described in the README.

use std::{fs, process::Command};
use tempfile::TempDir;

const AKON_BINARY: &str = "target/debug/akon";

fn write_config(temp_dir: &TempDir, lazy_mode: bool) {
    let config_path = temp_dir.path().join("config.toml");
    let bool_literal = if lazy_mode { "true" } else { "false" };
    let contents = format!(
        "server = \"vpn.example.com\"\nusername = \"lazy_user\"\nprotocol = \"f5\"\nlazy_mode = {bool_literal}\n\n[vpn]\nserver = \"vpn.example.com\"\nusername = \"lazy_user\"\nprotocol = \"f5\"\nlazy_mode = {bool_literal}\n"
    );

    fs::write(config_path, contents).expect("failed to write config.toml");
}

#[test]
fn test_lazy_mode_disabled_shows_help() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    write_config(&temp_dir, false);
    let state_path = temp_dir.path().join("state.json");

    let output = Command::new(AKON_BINARY)
        .env("AKON_CONFIG_DIR", temp_dir.path())
        .env("AKON_STATE_FILE", &state_path)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run akon binary");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit code 2 when lazy mode is disabled"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("VPN automatic authentication tool"),
        "expected help output when lazy mode is disabled"
    );
    assert!(
        output.stderr.is_empty(),
        "expected no stderr output when showing help"
    );
}

#[test]
fn test_lazy_mode_enabled_invokes_vpn_on() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    write_config(&temp_dir, true);
    let state_path = temp_dir.path().join("state.json");

    let output = Command::new(AKON_BINARY)
        .env("AKON_CONFIG_DIR", temp_dir.path())
        .env("AKON_STATE_FILE", &state_path)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run akon binary");

    assert!(
        !output.status.success(),
        "expected lazy mode to fail when prerequisites are missing"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Keyring error")
            || stderr.contains("OpenConnect is not installed")
            || stderr.contains("Failed to spawn"),
        "expected lazy mode failure to surface prerequisite error, stderr: {}",
        stderr
    );
}
