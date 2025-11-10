//! Authentication module
//!
//! Handles PIN storage, OTP secret storage, TOTP generation, and keyring operations.

pub mod base32;
pub mod hmac;

// Use mock keyring in test mode or CI environment
#[cfg(any(test, feature = "mock-keyring"))]
#[path = "keyring_mock.rs"]
pub mod keyring;

// Use real keyring in production
#[cfg(not(any(test, feature = "mock-keyring")))]
pub mod keyring;

pub mod password;
pub mod totp;
