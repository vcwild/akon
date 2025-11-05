//! Password generation module (PIN + OTP)
//!
//! This module provides complete VPN password generation by combining
//! the 4-digit PIN with the 6-digit TOTP token.

use crate::auth::{keyring, totp};
use crate::error::AkonError;
use crate::types::{OtpSecret, VpnPassword};

/// Generate the complete VPN password (PIN + OTP)
///
/// Retrieves the PIN and OTP secret from keyring, generates a fresh OTP,
/// and returns the complete 10-character password.
///
/// # Errors
///
/// Returns an error if:
/// - PIN is not found in keyring
/// - OTP secret is not found in keyring
/// - OTP generation fails
pub fn generate_password(username: &str) -> Result<VpnPassword, AkonError> {
    // Retrieve PIN from keyring
    let pin = keyring::retrieve_pin(username)?;

    // Retrieve OTP secret from keyring
    let otp_secret_str = keyring::retrieve_otp_secret(username)?;
    let otp_secret = OtpSecret::new(otp_secret_str);

    // Generate OTP token
    let otp_token = totp::generate_otp(&otp_secret, None)?;

    // Combine PIN + OTP
    Ok(VpnPassword::from_components(&pin, &otp_token))
}

/// Generate password with explicit credentials (for testing)
pub fn generate_password_from_credentials(
    pin: &crate::types::Pin,
    otp_secret: &OtpSecret,
    timestamp: Option<u64>,
) -> Result<VpnPassword, AkonError> {
    let otp_token = totp::generate_otp(otp_secret, timestamp)?;
    Ok(VpnPassword::from_components(pin, &otp_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Pin;

    #[test]
    fn test_generate_password_from_credentials() {
        let pin = Pin::new("1234".to_string()).unwrap();
        let otp_secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());

        // Generate with fixed timestamp for reproducibility
        let timestamp = 1609459200; // 2021-01-01 00:00:00 UTC
        let result = generate_password_from_credentials(&pin, &otp_secret, Some(timestamp));

        assert!(result.is_ok());
        let password = result.unwrap();

        // Password should be exactly 10 characters
        assert_eq!(password.expose().len(), 10);

        // Should start with PIN
        assert!(password.expose().starts_with("1234"));

        // All characters should be digits
        assert!(password.expose().chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_password_format() {
        let pin = Pin::new("9999".to_string()).unwrap();
        let otp_secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());

        let result = generate_password_from_credentials(&pin, &otp_secret, None);
        assert!(result.is_ok());

        let password = result.unwrap();
        let pwd_str = password.expose();

        // Verify format: 4-digit PIN + 6-digit OTP = 10 characters
        assert_eq!(pwd_str.len(), 10);
        assert!(pwd_str.starts_with("9999"));
        assert!(pwd_str.chars().all(|c| c.is_ascii_digit()));
    }
}
