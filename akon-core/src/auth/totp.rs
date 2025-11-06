//! TOTP (Time-based One-Time Password) generation
//!
//! Implements RFC 6238 TOTP with custom HMAC-SHA1 and Base32 decoding
//! to match auto-openconnect's algorithm exactly for cross-compatibility.

use crate::auth::{base32, hmac};
use crate::error::AkonError;
use crate::types::{OtpSecret, TotpToken};
use std::time::{SystemTime, UNIX_EPOCH};

/// Get HOTP counter from timestamp
///
/// Matches auto-openconnect's logic: `int(time.time() / 30)`
/// Uses integer division to match Python's behavior
fn get_hotp_counter(timestamp: Option<u64>) -> Result<u64, AkonError> {
    let ts = timestamp.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before Unix epoch")
            .as_secs()
    });

    Ok(ts / 30) // Integer division, matching Python
}

/// Generate OTP token from secret, matching auto-openconnect's algorithm
///
/// This function implements the exact same logic as auto-openconnect's
/// `lib.py::generate_otp()` function:
/// 1. Calculate HOTP counter from timestamp
/// 2. Decode Base32 secret (with custom padding and whitespace handling)
/// 3. Compute HMAC-SHA1
/// 4. Apply dynamic truncation (RFC 6238)
/// 5. Return 6-digit OTP
pub fn generate_otp(secret: &OtpSecret, timestamp: Option<u64>) -> Result<TotpToken, AkonError> {
    // Step 1: Get HOTP counter (timestamp / 30)
    let counter = get_hotp_counter(timestamp)?;

    // Step 2: Decode Base32 secret with custom logic
    let key_bytes = base32::decode_base32(secret.expose()).map_err(AkonError::Otp)?;

    // Step 3: Convert counter to big-endian bytes
    let counter_bytes = counter.to_be_bytes();

    // Step 4: Compute HMAC-SHA1
    let hmac_result = hmac::hmac_sha1(&key_bytes, &counter_bytes);

    // Step 5: Dynamic truncation (RFC 6238)
    let offset = (hmac_result[19] & 0x0f) as usize;
    let code = u32::from_be_bytes([
        hmac_result[offset],
        hmac_result[offset + 1],
        hmac_result[offset + 2],
        hmac_result[offset + 3],
    ]);

    // Step 6: Generate 6-digit OTP
    let otp = (code & 0x7fffffff) % 1_000_000;

    Ok(TotpToken::new(format!("{:06}", otp)))
}

/// Generate a TOTP token with default settings (for backward compatibility)
pub fn generate_totp_default(secret: &str) -> Result<TotpToken, AkonError> {
    let otp_secret = OtpSecret::new(secret.to_string());
    generate_otp(&otp_secret, None)
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
        let otp_secret = OtpSecret::new("INVALID!@#$".to_string());
        let result = generate_otp(&otp_secret, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_otp_fixed_timestamp() {
        // Test with fixed timestamp for reproducibility
        let otp_secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());
        let timestamp = 1609459200; // 2021-01-01 00:00:00 UTC

        let result = generate_otp(&otp_secret, Some(timestamp));
        assert!(result.is_ok());

        let token = result.unwrap();
        assert_eq!(token.expose().len(), 6);
        assert!(token.expose().chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_hotp_counter_calculation() {
        // Test that counter calculation matches Python's int(time / 30)
        let result = get_hotp_counter(Some(1609459200));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1609459200 / 30);

        // Test with different timestamps
        assert_eq!(get_hotp_counter(Some(0)).unwrap(), 0);
        assert_eq!(get_hotp_counter(Some(30)).unwrap(), 1);
        assert_eq!(get_hotp_counter(Some(60)).unwrap(), 2);
        assert_eq!(get_hotp_counter(Some(89)).unwrap(), 2);
        assert_eq!(get_hotp_counter(Some(90)).unwrap(), 3);
    }

    #[test]
    fn test_generate_otp_format() {
        let otp_secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());
        let result = generate_otp(&otp_secret, None);

        assert!(result.is_ok());
        let token = result.unwrap();

        // Verify format: exactly 6 digits
        assert_eq!(token.expose().len(), 6);
        assert!(token.expose().chars().all(|c| c.is_ascii_digit()));
    }
}
