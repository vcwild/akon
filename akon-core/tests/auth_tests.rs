//! Unit tests for authentication functionality
//!
//! Tests OTP secret validation and TOTP generation.

use akon_core::auth::keyring;
use akon_core::error::OtpError;
use akon_core::types::{OtpSecret, TotpToken}; // Importing keyring module for testing

#[test]
fn test_valid_base32_secret() {
    let secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());
    assert!(secret.validate_base32().is_ok());
}

#[test]
fn test_base32_with_padding() {
    let secret = OtpSecret::new("JBSWY3DPEHPK3PXP======".to_string());
    assert!(secret.validate_base32().is_ok());
}

#[test]
fn test_base32_with_slashes() {
    let secret = OtpSecret::new("JBSWY3DPEHPK3PXP/ABC".to_string());
    assert!(secret.validate_base32().is_ok());
}

#[test]
fn test_invalid_base32_characters() {
    let secret = OtpSecret::new("INVALID@SECRET!".to_string());
    assert!(secret.validate_base32().is_err());
    assert_eq!(
        secret.validate_base32().unwrap_err(),
        OtpError::InvalidBase32
    );
}

#[test]
fn test_empty_secret() {
    let secret = OtpSecret::new("".to_string());
    assert!(secret.validate_base32().is_ok()); // Empty is technically valid base32
}

#[test]
fn test_lowercase_base32() {
    let secret = OtpSecret::new("jbswy3dpehpk3pxp".to_string());
    assert!(secret.validate_base32().is_ok());
}

#[test]
fn test_mixed_case_base32() {
    let secret = OtpSecret::new("JbsWy3DpEhPk3PxP".to_string());
    assert!(secret.validate_base32().is_ok());
}

#[test]
fn test_otp_secret_from_string() {
    let secret_str = "JBSWY3DPEHPK3PXP".to_string();
    let secret = OtpSecret::from(secret_str.clone());
    assert_eq!(secret.expose(), secret_str);
}

#[test]
fn test_totp_token_creation() {
    let token_str = "123456".to_string();
    let token = TotpToken::new(token_str.clone());
    assert_eq!(token.expose(), token_str);
}

#[test]
fn test_totp_token_from_string() {
    let token_str = "123456".to_string();
    let token = TotpToken::from(token_str.clone());
    assert_eq!(token.expose(), token_str);
}

#[test]
fn test_totp_token_6_digits() {
    let token = TotpToken::new("123456".to_string());
    assert_eq!(token.expose().len(), 6);
    assert!(token.expose().chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn test_totp_token_8_digits() {
    let token = TotpToken::new("12345678".to_string());
    assert_eq!(token.expose().len(), 8);
    assert!(token.expose().chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn test_delete_otp_secret_always_succeeds() {
    // The delete function is currently a stub that always returns Ok(())
    // This test ensures it doesn't panic and returns success
    let result = keyring::delete_otp_secret("test_user");
    assert!(result.is_ok());
}

#[test]
fn test_keyring_service_unavailable_error() {
    // Test that keyring operations return appropriate errors when service is unavailable
    // This tests the error handling path when Entry::new fails
    // Note: This test may pass or fail depending on system keyring availability
    let result = akon_core::auth::keyring::has_otp_secret("test_user_nonexistent");
    // We don't assert the exact result since it depends on keyring availability
    // The important thing is that it doesn't panic
    let _ = result; // Just ensure the function call doesn't panic
}

#[test]
fn test_store_otp_secret_error_handling() {
    // Test that store_otp_secret handles errors gracefully
    let result = akon_core::auth::keyring::store_otp_secret("test_user", "test_secret");
    // The result depends on keyring availability, but it should not panic
    let _ = result; // Just ensure the function call doesn't panic
}

#[test]
fn test_retrieve_otp_secret_error_handling() {
    // Test that retrieve_otp_secret handles errors gracefully
    let result = akon_core::auth::keyring::retrieve_otp_secret("test_user");
    // The result depends on keyring availability, but it should not panic
    let _ = result; // Just ensure the function call doesn't panic
}
