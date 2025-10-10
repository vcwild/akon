//! TOTP (Time-based One-Time Password) generation
//!
//! Implements RFC 6238 TOTP using the totp-lite crate for secure
//! OTP token generation from stored secrets.

use crate::error::{AkonError, OtpError};
use crate::types::TotpToken;
use base32::Alphabet;
use totp_lite::{Sha1, Sha256, Sha512};

/// Hash algorithm for TOTP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Default for HashAlgorithm {
    fn default() -> Self {
        Self::Sha1
    }
}

/// Generate a TOTP token from a Base32-encoded secret
pub fn generate_totp(
    secret: &str,
    algorithm: HashAlgorithm,
    digits: u32,
) -> Result<TotpToken, AkonError> {
    // Validate the secret is valid Base32
    if !is_valid_base32(secret) {
        return Err(AkonError::Otp(OtpError::InvalidBase32));
    }

    // Decode Base32 secret to bytes
    let secret_bytes = base32::decode(Alphabet::RFC4648 { padding: false }, secret)
        .ok_or(AkonError::Otp(OtpError::InvalidBase32))?;

    let time_step = 30; // RFC 6238 default

    // Get current time in seconds since Unix epoch
    use std::time::{SystemTime, UNIX_EPOCH};
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AkonError::Otp(OtpError::TimeError))?
        .as_secs();

    let token = match algorithm {
        HashAlgorithm::Sha1 => {
            totp_lite::totp_custom::<Sha1>(time_step, digits, &secret_bytes, current_time)
        }
        HashAlgorithm::Sha256 => {
            totp_lite::totp_custom::<Sha256>(time_step, digits, &secret_bytes, current_time)
        }
        HashAlgorithm::Sha512 => {
            totp_lite::totp_custom::<Sha512>(time_step, digits, &secret_bytes, current_time)
        }
    };

    Ok(TotpToken::new(token))
}

/// Generate a TOTP token with default settings (SHA1, 6 digits)
pub fn generate_totp_default(secret: &str) -> Result<TotpToken, AkonError> {
    generate_totp(secret, HashAlgorithm::Sha1, 6)
}

/// Validate that a string contains only valid Base32 characters
fn is_valid_base32(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_totp_default() {
        // Test with RFC 6238 test vector
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"; // Test secret
        let result = generate_totp_default(secret);
        assert!(result.is_ok());

        let token = result.unwrap();
        // Token should be 6 digits
        assert_eq!(token.expose().len(), 6);
        assert!(token.expose().chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_invalid_base32() {
        let invalid_secrets = vec![
            "INVALID!",
            "lowercase",
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ!", // Contains exclamation
        ];

        for secret in invalid_secrets {
            let result = generate_totp_default(secret);
            assert!(matches!(
                result,
                Err(AkonError::Otp(OtpError::InvalidBase32))
            ));
        }
    }

    #[test]
    fn test_valid_base32() {
        let valid_secrets = vec![
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ=", // With padding
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567",  // All valid chars
        ];

        for secret in valid_secrets {
            assert!(is_valid_base32(secret));
        }
    }
}
