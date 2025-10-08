//! Integration tests for keyring operations
//!
//! Tests keyring store/retrieve/has/delete operations using the actual
//! GNOME Keyring backend. These tests require a running GNOME Keyring daemon.

use akon_core::auth::keyring;
use akon_core::error::AkonError;

const TEST_USERNAME: &str = "__akon_test_user__";
const TEST_SECRET: &str = "JBSWY3DPEHPK3PXP";

#[test]
#[ignore] // Keyring tests require system keyring and may hang - run with `cargo test -- --ignored`
fn test_keyring_store_and_retrieve() {
    // This test requires a working GNOME Keyring or system keyring
    // Run with: cargo test -- --ignored test_keyring_store_and_retrieve

    // Clean up any existing test data
    let _ = keyring::delete_otp_secret(TEST_USERNAME);

    // Test storing a secret
    keyring::store_otp_secret(TEST_USERNAME, TEST_SECRET)
        .expect("Failed to store secret");

    // Test checking if secret exists
    let exists = keyring::has_otp_secret(TEST_USERNAME)
        .expect("Failed to check secret existence");
    assert!(exists, "Secret should exist after storing");

    // Test retrieving the secret
    let retrieved = keyring::retrieve_otp_secret(TEST_USERNAME)
        .expect("Failed to retrieve secret");
    assert_eq!(retrieved, TEST_SECRET, "Retrieved secret should match stored secret");

    // Clean up
    keyring::delete_otp_secret(TEST_USERNAME)
        .expect("Failed to delete test secret");
}

#[test]
#[ignore] // Keyring tests require system keyring and may hang - run with `cargo test -- --ignored`
fn test_keyring_has_nonexistent() {
    // This test requires a working GNOME Keyring or system keyring
    // Run with: cargo test -- --ignored test_keyring_has_nonexistent

    let nonexistent_username = "__akon_nonexistent__";

    // Should not exist
    let exists = keyring::has_otp_secret(nonexistent_username)
        .expect("Failed to check secret existence");
    assert!(!exists, "Nonexistent secret should not exist");

    // Should fail to retrieve
    let result = keyring::retrieve_otp_secret(nonexistent_username);
    assert!(result.is_err(), "Retrieving nonexistent secret should fail");
}
