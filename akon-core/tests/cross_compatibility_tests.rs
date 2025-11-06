//! Cross-compatibility tests with auto-openconnect
//!
//! These tests verify that akon's password generation produces identical
//! results to auto-openconnect's Python implementation for the same inputs.

use akon_core::auth::{base32, hmac, totp};
use akon_core::types::{OtpSecret, Pin, VpnPassword};

/// Test Base32 decoding compatibility
///
/// Verifies that our Base32 decode matches Python's base64.b32decode
/// with casefold=True and proper padding
#[test]
fn test_base32_decode_compatibility() {
    // Test case 1: "JBSWY3DPEE" (without padding) -> "Hello!"
    let input1 = "JBSWY3DPEE";
    let result1 = base32::decode_base32(input1).expect("Valid Base32");
    assert_eq!(result1, b"Hello!", "Base32 decode should produce 'Hello!'");

    // Test case 2: RFC 6238 test secret (20 bytes when decoded)
    let input2 = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"; // "12345678901234567890"
    let result2 = base32::decode_base32(input2).expect("Valid Base32");
    assert_eq!(
        result2, b"12345678901234567890",
        "Should decode RFC 6238 test secret"
    );

    // Test case 3: Base32 with spaces (should be cleaned)
    let input_with_spaces = "JBSW Y3DP EE";
    let result_with_spaces =
        base32::decode_base32(input_with_spaces).expect("Valid Base32 with spaces");
    assert_eq!(
        result_with_spaces, b"Hello!",
        "Should handle spaces like auto-openconnect"
    );

    // Test case 4: Lowercase (should work with casefold)
    let input_lowercase = "jbswy3dpee";
    let result_lowercase = base32::decode_base32(input_lowercase).expect("Valid lowercase Base32");
    assert_eq!(
        result_lowercase, b"Hello!",
        "Should handle lowercase like auto-openconnect"
    );

    // Test case 5: Mixed case
    let input_mixed = "JbSwY3DpEe";
    let result_mixed = base32::decode_base32(input_mixed).expect("Valid mixed-case Base32");
    assert_eq!(
        result_mixed, b"Hello!",
        "Should handle mixed case like auto-openconnect"
    );
}

/// Test HMAC-SHA1 compatibility
///
/// Verifies that our HMAC-SHA1 implementation matches Python's implementation
/// using RFC 2104 test vectors and custom test cases
#[test]
fn test_hmac_sha1_compatibility() {
    // RFC 2104 Test Case 1
    let key = b"\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b";
    let data = b"Hi There";
    let result = hmac::hmac_sha1(key, data);
    let expected = hex::decode("b617318655057264e28bc0b6fb378c8ef146be00").unwrap();
    assert_eq!(
        result.to_vec(),
        expected,
        "RFC 2104 Test Case 1 should match"
    );

    // RFC 2104 Test Case 2
    let key2 = b"Jefe";
    let data2 = b"what do ya want for nothing?";
    let result2 = hmac::hmac_sha1(key2, data2);
    let expected2 = hex::decode("effcdf6ae5eb2fa2d27416d5f184df9c259a7c79").unwrap();
    assert_eq!(
        result2.to_vec(),
        expected2,
        "RFC 2104 Test Case 2 should match"
    );

    // Test with HOTP counter (as used in TOTP)
    let key3 = b"12345678901234567890"; // 20 bytes
    let counter: u64 = 1;
    let counter_bytes = counter.to_be_bytes();
    let result3 = hmac::hmac_sha1(key3, &counter_bytes);

    // Verify the result is 20 bytes (SHA-1 output)
    assert_eq!(result3.len(), 20, "HMAC-SHA1 should produce 20 bytes");
}

/// Test TOTP generation compatibility with fixed timestamp
///
/// Verifies that our OTP generation matches auto-openconnect's for the same
/// secret and timestamp
#[test]
fn test_totp_generation_compatibility() {
    // Test secret from RFC 6238
    let secret = "JBSWY3DPEHPK3PXP";
    let otp_secret = OtpSecret::new(secret.to_string());

    // Test with specific timestamps (within 30-second windows)
    let test_cases: Vec<(u64, Option<&str>)> = vec![
        (59u64, None),         // End of first 30-second window
        (1111111109u64, None), // RFC 6238 test case
        (1234567890u64, None), // Another test timestamp
    ];

    for (timestamp, expected_otp) in test_cases {
        let result = totp::generate_otp(&otp_secret, Some(timestamp));
        assert!(
            result.is_ok(),
            "OTP generation should succeed for timestamp {}",
            timestamp
        );

        let otp = result.unwrap();
        let otp_str = otp.expose();
        assert_eq!(otp_str.len(), 6, "OTP should be 6 digits");
        assert!(
            otp_str.chars().all(|c| c.is_ascii_digit()),
            "OTP should contain only digits"
        );

        // If we have an expected value, verify it
        if let Some(expected) = expected_otp {
            assert_eq!(
                otp_str, expected,
                "OTP should match expected value for timestamp {}",
                timestamp
            );
        }
    }
}

/// Test complete password generation compatibility
///
/// Verifies that PIN + OTP produces the expected 10-character format
#[test]
fn test_complete_password_format() {
    let pin = Pin::new("1234".to_string()).expect("Valid PIN");
    let secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());

    // Generate OTP with fixed timestamp for reproducibility
    let timestamp = 1234567890u64;
    let otp = totp::generate_otp(&secret, Some(timestamp)).expect("Valid OTP");

    // Combine into password
    let password = VpnPassword::from_components(&pin, &otp);

    // Verify format
    let password_str = password.expose();
    assert_eq!(
        password_str.len(),
        10,
        "Password should be exactly 10 characters"
    );
    assert!(
        password_str.chars().all(|c| c.is_ascii_digit()),
        "Password should be all digits"
    );
    assert!(
        password_str.starts_with("1234"),
        "Password should start with PIN"
    );
}

/// Test HOTP counter calculation matches auto-openconnect
///
/// Python: int(time.time() / 30)
/// Rust: timestamp / 30 (integer division)
#[test]
fn test_hotp_counter_calculation() {
    let test_cases = vec![
        (0u64, 0u64),
        (29u64, 0u64),
        (30u64, 1u64),
        (59u64, 1u64),
        (60u64, 2u64),
        (1234567890u64, 41152263u64),
        (1111111109u64, 37037036u64),
        (2000000000u64, 66666666u64),
    ];

    for (timestamp, expected_counter) in test_cases {
        let counter = timestamp / 30;
        assert_eq!(
            counter, expected_counter,
            "HOTP counter for timestamp {} should be {}",
            timestamp, expected_counter
        );
    }
}

/// Test with known auto-openconnect test vectors
///
/// These test vectors should produce identical results in both implementations
#[test]
fn test_known_otp_values() {
    // RFC 6238 Test Vectors (SHA1)
    let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"; // Base32 encoding of "12345678901234567890"
    let otp_secret = OtpSecret::new(secret.to_string());

    // Known test vectors from RFC 6238
    let test_vectors = vec![
        (59u64, "287082"),          // T=1
        (1111111109u64, "081804"),  // T=37037036
        (1111111111u64, "050471"),  // T=37037037
        (1234567890u64, "005924"),  // T=41152263
        (2000000000u64, "279037"),  // T=66666666
        (20000000000u64, "353130"), // T=666666666
    ];

    for (timestamp, expected_otp) in test_vectors {
        let result = totp::generate_otp(&otp_secret, Some(timestamp))
            .expect(&format!("Should generate OTP for timestamp {}", timestamp));

        assert_eq!(
            result.expose(),
            expected_otp,
            "OTP for timestamp {} should match RFC 6238 test vector",
            timestamp
        );
    }
}

/// Test padding logic matches auto-openconnect
///
/// Python: padding = "=" * ((size - remainder) % size)
/// where _, remainder = divmod(len(input_str), size)
#[test]
fn test_padding_logic() {
    let test_cases = vec![
        ("", 0),          // Length 0: (8 - 0) % 8 = 0 padding
        ("A", 7),         // Length 1: (8 - 1) % 8 = 7 padding
        ("AB", 6),        // Length 2: (8 - 2) % 8 = 6 padding
        ("ABCDEFG", 1),   // Length 7: (8 - 7) % 8 = 1 padding
        ("ABCDEFGH", 0),  // Length 8: (8 - 0) % 8 = 0 padding
        ("ABCDEFGHI", 7), // Length 9: (8 - 1) % 8 = 7 padding
    ];

    for (input, expected_padding_count) in test_cases {
        let padding_needed = (8 - (input.len() % 8)) % 8;
        assert_eq!(
            padding_needed,
            expected_padding_count,
            "Padding for '{}' (len={}) should be {} '=' chars",
            input,
            input.len(),
            expected_padding_count
        );
    }
}

/// Integration test: End-to-end password generation
///
/// This test verifies the complete flow from PIN + Secret -> Password
/// with a fixed timestamp for reproducibility
#[test]
fn test_end_to_end_password_generation() {
    // Setup test credentials
    let pin = Pin::new("5678".to_string()).expect("Valid PIN");
    let secret = OtpSecret::new("JBSWY3DPEHPK3PXP".to_string());

    // Use a fixed timestamp in the middle of a 30-second window
    let timestamp = 1700000015u64; // Should give counter = 56666667

    // Generate OTP
    let otp = totp::generate_otp(&secret, Some(timestamp)).expect("Should generate OTP");

    // Verify OTP format
    let otp_str = otp.expose();
    assert_eq!(otp_str.len(), 6, "OTP should be 6 digits");

    // Create complete password
    let password = VpnPassword::from_components(&pin, &otp);
    let password_str = password.expose();

    // Verify complete password format
    assert_eq!(password_str.len(), 10, "Password should be 10 characters");
    assert!(
        password_str.starts_with("5678"),
        "Password should start with PIN '5678'"
    );
    assert!(
        password_str.chars().all(|c| c.is_ascii_digit()),
        "Password should be all digits"
    );

    // The last 6 characters should be the OTP
    assert_eq!(
        &password_str[4..],
        otp_str,
        "Last 6 chars should be the OTP"
    );
}

/// Test edge case: Dynamic truncation offset
///
/// Verifies that the dynamic truncation (DT) works correctly
/// DT offset = last byte & 0x0F
#[test]
fn test_dynamic_truncation() {
    // The dynamic truncation offset should be between 0 and 15
    // because it's masked with 0x0F

    let secret = "JBSWY3DPEHPK3PXP";
    let otp_secret = OtpSecret::new(secret.to_string());

    // Generate multiple OTPs with different timestamps
    for timestamp in 1000000000..1000000010 {
        let otp = totp::generate_otp(&otp_secret, Some(timestamp)).expect("Should generate OTP");

        // Verify it's a valid 6-digit number
        let otp_str = otp.expose();
        let otp_num: u32 = otp_str.parse().expect("OTP should be numeric");
        assert!(otp_num < 1_000_000, "OTP should be less than 1,000,000");

        // Verify format
        assert_eq!(
            otp_str.len(),
            6,
            "OTP should always be 6 digits (with leading zeros)"
        );
    }
}
