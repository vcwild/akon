//! Custom Base32 decoding to match auto-openconnect behavior
//!
//! This module implements Base32 decoding with the exact same logic as
//! auto-openconnect's `lib.py` to ensure cross-compatibility:
//! 1. Remove all whitespace characters
//! 2. Apply padding to 8-character boundaries
//! 3. Decode with casefold=true (case-insensitive)

use crate::error::OtpError;

/// Clean whitespace from input string
///
/// Matches auto-openconnect's `clean()` function
fn clean(input: &str) -> String {
    input.replace(' ', "")
}

/// Pad input string to 8-character boundaries
///
/// Matches auto-openconnect's `pad()` function
/// Formula: padding_length = (8 - (len % 8)) % 8
fn pad(input: &str) -> String {
    let padding_len = (8 - (input.len() % 8)) % 8;
    format!("{}{}", input, "=".repeat(padding_len))
}

/// Decode Base32 string to bytes, matching auto-openconnect's algorithm
///
/// This implementation follows the exact same steps as auto-openconnect:
/// 1. Remove whitespace with `clean()`
/// 2. Add padding with `pad()`
/// 3. Decode using base32 with casefold=true
pub fn decode_base32(input: &str) -> Result<Vec<u8>, OtpError> {
    // Step 1: Remove whitespace
    let cleaned = clean(input);

    // Step 2: Apply padding
    let padded = pad(&cleaned);

    // Step 3: Decode with casefold (case-insensitive)
    // Using data_encoding crate which supports case-insensitive decoding
    use data_encoding::BASE32;

    BASE32
        .decode(padded.to_uppercase().as_bytes())
        .map_err(|_| OtpError::InvalidBase32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_removes_spaces() {
        assert_eq!(clean("JBSW Y3DP EHPK 3PXP"), "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn test_clean_no_spaces() {
        assert_eq!(clean("JBSWY3DPEHPK3PXP"), "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn test_pad_no_padding_needed() {
        // Length 16, already multiple of 8
        assert_eq!(pad("JBSWY3DPEHPK3PXP"), "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn test_pad_needs_padding() {
        // Length 14, needs 2 padding chars to reach 16
        assert_eq!(pad("JBSWY3DPEHPK3P"), "JBSWY3DPEHPK3P==");
    }

    #[test]
    fn test_pad_formula() {
        // Test padding formula: (8 - (len % 8)) % 8
        assert_eq!(pad("A").len(), 8); // len=1, pad=7: "A======="
        assert_eq!(pad("AB").len(), 8); // len=2, pad=6: "AB======"
        assert_eq!(pad("ABC").len(), 8); // len=3, pad=5: "ABC====="
        assert_eq!(pad("ABCD").len(), 8); // len=4, pad=4: "ABCD===="
        assert_eq!(pad("ABCDE").len(), 8); // len=5, pad=3: "ABCDE==="
        assert_eq!(pad("ABCDEF").len(), 8); // len=6, pad=2: "ABCDEF=="
        assert_eq!(pad("ABCDEFG").len(), 8); // len=7, pad=1: "ABCDEFG="
        assert_eq!(pad("ABCDEFGH").len(), 8); // len=8, pad=0: "ABCDEFGH"
    }

    #[test]
    fn test_decode_base32_valid() {
        let result = decode_base32("JBSWY3DPEHPK3PXP");
        assert!(result.is_ok());
        let bytes = result.unwrap();
        // "JBSWY3DPEHPK3PXP" decodes to "Hello!??" in ASCII
        assert_eq!(bytes.len(), 10);
    }

    #[test]
    fn test_decode_base32_with_spaces() {
        let result = decode_base32("JBSW Y3DP EHPK 3PXP");
        assert!(result.is_ok());
        // Should decode to same as without spaces
        let with_spaces = decode_base32("JBSW Y3DP EHPK 3PXP").unwrap();
        let without_spaces = decode_base32("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(with_spaces, without_spaces);
    }

    #[test]
    fn test_decode_base32_lowercase() {
        // Should work with lowercase (casefold=true)
        let upper = decode_base32("JBSWY3DPEHPK3PXP").unwrap();
        let lower = decode_base32("jbswy3dpehpk3pxp").unwrap();
        let mixed = decode_base32("JbSwY3DpEhPk3PxP").unwrap();

        assert_eq!(upper, lower);
        assert_eq!(upper, mixed);
    }

    #[test]
    fn test_decode_base32_invalid() {
        // Invalid Base32 characters
        let result = decode_base32("INVALID@CHARS!");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), OtpError::InvalidBase32);
    }
}
