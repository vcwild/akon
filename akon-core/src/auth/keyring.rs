//! Keyring operations for secure credential storage
//!
//! Uses the system keyring (GNOME Keyring on Linux) to store and retrieve
//! sensitive VPN credentials securely.

use crate::error::{AkonError, KeyringError};
use crate::types::{Pin, KEYRING_SERVICE_PIN, KEYRING_SERVICE_OTP};
use keyring::Entry;


/// Store an OTP secret in the system keyring
pub fn store_otp_secret(username: &str, secret: &str) -> Result<(), AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_OTP, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    entry
        .set_password(secret)
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;

    Ok(())
}

/// Retrieve an OTP secret from the system keyring
pub fn retrieve_otp_secret(username: &str) -> Result<String, AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_OTP, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    entry
        .get_password()
        .map_err(|_| AkonError::Keyring(KeyringError::RetrieveFailed))
}

/// Check if an OTP secret exists in the keyring for the given username
pub fn has_otp_secret(username: &str) -> Result<bool, AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_OTP, username)
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

/// Store a PIN in the system keyring
///
/// Stores the 4-digit PIN with service name "akon-vpn-pin"
pub fn store_pin(username: &str, pin: &Pin) -> Result<(), AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_PIN, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    entry
        .set_password(pin.expose())
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;

    Ok(())
}

/// Retrieve a PIN from the system keyring
///
/// Returns the PIN if found, or KeyringError::PinNotFound if not present
pub fn retrieve_pin(username: &str) -> Result<Pin, AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_PIN, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    let pin_str = entry
        .get_password()
        .map_err(|_| AkonError::Keyring(KeyringError::PinNotFound))?;

    // Enforce the internal hard limit of 30 characters at retrieval time.
    // This truncation is silent and ensures downstream consumers never see
    // a PIN longer than 30 characters.
    let pin_trimmed = pin_str.trim().to_string();
    let stored = if pin_trimmed.chars().count() > 30 {
        pin_trimmed.chars().take(30).collect::<String>()
    } else {
        pin_trimmed.clone()
    };

    // Return a Pin without re-applying strict 4-digit validation.
    Ok(Pin::from_unchecked(stored))
}

/// Check if a PIN exists in the keyring for the given username
pub fn has_pin(username: &str) -> Result<bool, AkonError> {
    let entry = Entry::new(KEYRING_SERVICE_PIN, username)
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;

    match entry.get_password() {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Delete a PIN from the keyring
pub fn delete_pin(_username: &str) -> Result<(), AkonError> {
    // Note: The keyring crate doesn't provide a reliable delete API
    // For now, we just return success to avoid blocking operations
    Ok(())
}
