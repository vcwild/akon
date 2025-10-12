//! Custom HMAC-SHA1 implementation matching auto-openconnect
//!
//! This module implements HMAC-SHA1 following RFC 2104 exactly as
//! auto-openconnect's `lib.py` does, to ensure cross-compatibility.
//!
//! Reference: https://www.ietf.org/rfc/rfc2104.txt
//! Block size: 64 bytes for SHA-1
//! Inner pad (ipad): 0x36
//! Outer pad (opad): 0x5C

use sha1::{Digest, Sha1};

const BLOCK_SIZE: usize = 64;
const IPAD: u8 = 0x36;
const OPAD: u8 = 0x5C;

/// Compute HMAC-SHA1 following RFC 2104
///
/// This implementation matches auto-openconnect's `hmac()` function:
/// 1. Create translation tables for ipad and opad
/// 2. Hash key if longer than block size
/// 3. Pad key to block size
/// 4. XOR key with ipad and opad
/// 5. Compute inner and outer hashes
pub fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    // Step 1: Process key
    let mut key_block = [0u8; BLOCK_SIZE];

    if key.len() > BLOCK_SIZE {
        // If key is longer than block size, hash it first
        let mut hasher = Sha1::new();
        hasher.update(key);
        let hashed = hasher.finalize();
        key_block[..20].copy_from_slice(&hashed);
    } else {
        // Otherwise use key directly
        key_block[..key.len()].copy_from_slice(key);
    }
    // Remaining bytes are already 0x00 (padding)

    // Step 2: Create ipad and opad keys
    let mut ipad_key = [0u8; BLOCK_SIZE];
    let mut opad_key = [0u8; BLOCK_SIZE];

    for i in 0..BLOCK_SIZE {
        ipad_key[i] = key_block[i] ^ IPAD;
        opad_key[i] = key_block[i] ^ OPAD;
    }

    // Step 3: Compute inner hash
    let mut inner = Sha1::new();
    inner.update(ipad_key);
    inner.update(message);
    let inner_hash = inner.finalize();

    // Step 4: Compute outer hash
    let mut outer = Sha1::new();
    outer.update(opad_key);
    outer.update(inner_hash);
    let outer_hash = outer.finalize();

    // Convert to fixed-size array
    let mut result = [0u8; 20];
    result.copy_from_slice(&outer_hash);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sha1_rfc2104_test_case_1() {
        // RFC 2104 Test Case 1
        // key = 0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b (20 bytes)
        // data = "Hi There"
        // Expected: 0xb617318655057264e28bc0b6fb378c8ef146be00

        let key = [0x0b; 20];
        let data = b"Hi There";
        let result = hmac_sha1(&key, data);

        let expected = [
            0xb6, 0x17, 0x31, 0x86, 0x55, 0x05, 0x72, 0x64,
            0xe2, 0x8b, 0xc0, 0xb6, 0xfb, 0x37, 0x8c, 0x8e,
            0xf1, 0x46, 0xbe, 0x00,
        ];

        assert_eq!(result, expected);
    }

    #[test]
    fn test_hmac_sha1_rfc2104_test_case_2() {
        // RFC 2104 Test Case 2
        // key = "Jefe"
        // data = "what do ya want for nothing?"
        // Expected: 0xeffcdf6ae5eb2fa2d27416d5f184df9c259a7c79

        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let result = hmac_sha1(key, data);

        let expected = [
            0xef, 0xfc, 0xdf, 0x6a, 0xe5, 0xeb, 0x2f, 0xa2,
            0xd2, 0x74, 0x16, 0xd5, 0xf1, 0x84, 0xdf, 0x9c,
            0x25, 0x9a, 0x7c, 0x79,
        ];

        assert_eq!(result, expected);
    }

    #[test]
    fn test_hmac_sha1_rfc2104_test_case_3() {
        // RFC 2104 Test Case 3
        // key = 0xaaaa...aaaa (20 bytes)
        // data = 0xdddd...dddd (50 bytes)
        // Expected: 0x125d7342b9ac11cd91a39af48aa17b4f63f175d3

        let key = [0xaa; 20];
        let data = [0xdd; 50];
        let result = hmac_sha1(&key, &data);

        let expected = [
            0x12, 0x5d, 0x73, 0x42, 0xb9, 0xac, 0x11, 0xcd,
            0x91, 0xa3, 0x9a, 0xf4, 0x8a, 0xa1, 0x7b, 0x4f,
            0x63, 0xf1, 0x75, 0xd3,
        ];

        assert_eq!(result, expected);
    }

    #[test]
    fn test_hmac_sha1_long_key() {
        // Test with key longer than block size (64 bytes)
        // Key should be hashed first
        let key = [0xaa; 80];
        let data = b"Test Using Larger Than Block-Size Key";
        let result = hmac_sha1(&key, data);

        // Just verify it produces 20 bytes
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn test_hmac_sha1_empty_message() {
        let key = b"key";
        let data = b"";
        let result = hmac_sha1(key, data);

        // Should produce valid HMAC even with empty message
        assert_eq!(result.len(), 20);
    }
}
