//! Authentication module
//!
//! Handles PIN storage, OTP secret storage, TOTP generation, and keyring operations.

pub mod base32;
pub mod hmac;
pub mod keyring;
pub mod password;
pub mod totp;
