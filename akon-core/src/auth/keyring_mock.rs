//! Mock keyring implementation for testing
//!
//! Provides an in-memory keyring implementation that doesn't require
//! system keyring access. Used in CI environments and for testing.

use crate::error::{AkonError, KeyringError};
use crate::types::Pin;
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref MOCK_KEYRING: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

/// Generate a key for the mock keyring
fn make_key(service: &str, username: &str) -> String {
    format!("{}:{}", service, username)
}

/// Service name used for storing credentials in the keyring (legacy)
const SERVICE_NAME: &str = "akon-vpn";

/// Service name for PIN storage
const SERVICE_NAME_PIN: &str = "akon-vpn-pin";

/// Store an OTP secret in the mock keyring
pub fn store_otp_secret(username: &str, secret: &str) -> Result<(), AkonError> {
    let key = make_key(SERVICE_NAME, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.insert(key, secret.to_string());
    Ok(())
}

/// Retrieve an OTP secret from the mock keyring
pub fn retrieve_otp_secret(username: &str) -> Result<String, AkonError> {
    let key = make_key(SERVICE_NAME, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::RetrieveFailed))?;
    keyring
        .get(&key)
        .cloned()
        .ok_or(AkonError::Keyring(KeyringError::RetrieveFailed))
}

/// Check if an OTP secret exists in the mock keyring for the given username
pub fn has_otp_secret(username: &str) -> Result<bool, AkonError> {
    let key = make_key(SERVICE_NAME, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;
    Ok(keyring.contains_key(&key))
}

/// Delete an OTP secret from the mock keyring
pub fn delete_otp_secret(username: &str) -> Result<(), AkonError> {
    let key = make_key(SERVICE_NAME, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.remove(&key);
    Ok(())
}

/// Store a PIN in the mock keyring
pub fn store_pin(username: &str, pin: &Pin) -> Result<(), AkonError> {
    let key = make_key(SERVICE_NAME_PIN, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.insert(key, pin.expose().to_string());
    Ok(())
}

/// Retrieve a PIN from the mock keyring
pub fn retrieve_pin(username: &str) -> Result<Pin, AkonError> {
    let key = make_key(SERVICE_NAME_PIN, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::PinNotFound))?;
    let pin_str = keyring
        .get(&key)
        .cloned()
        .ok_or(AkonError::Keyring(KeyringError::PinNotFound))?;
    Pin::new(pin_str).map_err(AkonError::Otp)
}

/// Check if a PIN exists in the mock keyring for the given username
pub fn has_pin(username: &str) -> Result<bool, AkonError> {
    let key = make_key(SERVICE_NAME_PIN, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;
    Ok(keyring.contains_key(&key))
}

/// Delete a PIN from the mock keyring
pub fn delete_pin(username: &str) -> Result<(), AkonError> {
    let key = make_key(SERVICE_NAME_PIN, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.remove(&key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_store_and_retrieve() {
        let username = "test_user_mock";
        let secret = "JBSWY3DPEHPK3PXP";

        // Clean up first
        let _ = delete_otp_secret(username);

        // Store
        store_otp_secret(username, secret).expect("Failed to store secret");

        // Retrieve
        let retrieved = retrieve_otp_secret(username).expect("Failed to retrieve secret");
        assert_eq!(retrieved, secret);

        // Clean up
        delete_otp_secret(username).expect("Failed to delete secret");
    }

    #[test]
    fn test_mock_pin_operations() {
        let username = "test_user_pin_mock";
        let pin = Pin::new("1234".to_string()).expect("Valid PIN");

        // Clean up first
        let _ = delete_pin(username);

        // Store
        store_pin(username, &pin).expect("Failed to store PIN");

        // Check exists
        assert!(has_pin(username).expect("Failed to check PIN"));

        // Retrieve
        let retrieved = retrieve_pin(username).expect("Failed to retrieve PIN");
        assert_eq!(retrieved.expose(), "1234");

        // Clean up
        delete_pin(username).expect("Failed to delete PIN");
        assert!(!has_pin(username).expect("Failed to check PIN after delete"));
    }
}
