//! Unit tests for authentication functionality
//!
//! Tests OTP secret validation and TOTP generation.

use akon_core::types::OtpSecret;
use akon_core::error::OtpError;

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
    assert_eq!(secret.validate_base32().unwrap_err(), OtpError::InvalidBase32);
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
