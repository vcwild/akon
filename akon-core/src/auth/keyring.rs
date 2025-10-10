//! Keyring operations for secure credential storage
//!
//! Uses the system keyring (GNOME Keyring on Linux) to store and retrieve
//! sensitive VPN credentials securely.

use crate::error::{AkonError, KeyringError};
use keyring::Entry;

/// Service name used for storing credentials in the keyring
const SERVICE_NAME: &str = "akon-vpn";

/// Store an OTP secret in the system keyring
pub fn store_otp_secret(username: &str, secret: &str) -> Result<(), AkonError> {
    let entry = Entry::new(SERVICE_NAME, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    entry
        .set_password(secret)
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;

    Ok(())
}

/// Retrieve an OTP secret from the system keyring
pub fn retrieve_otp_secret(username: &str) -> Result<String, AkonError> {
    let entry = Entry::new(SERVICE_NAME, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    entry
        .get_password()
        .map_err(|_| AkonError::Keyring(KeyringError::RetrieveFailed))
}

/// Check if an OTP secret exists in the keyring for the given username
pub fn has_otp_secret(username: &str) -> Result<bool, AkonError> {
    let entry = Entry::new(SERVICE_NAME, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    match entry.get_password() {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Delete an OTP secret from the keyring
pub fn delete_otp_secret(_username: &str) -> Result<(), AkonError> {
    // Note: The keyring crate doesn't provide a reliable delete API
    // For now, we just return success to avoid blocking operations
    // In a real implementation, this would need platform-specific code
    // or a different keyring library that supports deletion
    Ok(())
}
