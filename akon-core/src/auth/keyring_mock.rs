//! Mock keyring implementation for testing
//!
//! Provides an in-memory keyring implementation that doesn't require
//! system keyring access. Used in CI environments and for testing.

use crate::error::{AkonError, KeyringError};
use crate::types::{Pin, KEYRING_SERVICE_OTP, KEYRING_SERVICE_PIN};
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref MOCK_KEYRING: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

/// Generate a key for the mock keyring
fn make_key(service: &str, username: &str) -> String {
    format!("{}:{}", service, username)
}

/// Store an OTP secret in the mock keyring
pub fn store_otp_secret(username: &str, secret: &str) -> Result<(), AkonError> {
    let key = make_key(KEYRING_SERVICE_OTP, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.insert(key, secret.to_string());
    Ok(())
}

/// Retrieve an OTP secret from the mock keyring
pub fn retrieve_otp_secret(username: &str) -> Result<String, AkonError> {
    let key = make_key(KEYRING_SERVICE_OTP, username);
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
    let key = make_key(KEYRING_SERVICE_OTP, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;
    Ok(keyring.contains_key(&key))
}

/// Delete an OTP secret from the mock keyring
pub fn delete_otp_secret(username: &str) -> Result<(), AkonError> {
    let key = make_key(KEYRING_SERVICE_OTP, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.remove(&key);
    Ok(())
}

/// Store a PIN in the mock keyring
pub fn store_pin(username: &str, pin: &Pin) -> Result<(), AkonError> {
    let key = make_key(KEYRING_SERVICE_PIN, username);
    let mut keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::StoreFailed))?;
    keyring.insert(key, pin.expose().to_string());
    Ok(())
}

/// Retrieve a PIN from the mock keyring
pub fn retrieve_pin(username: &str) -> Result<Pin, AkonError> {
    let key = make_key(KEYRING_SERVICE_PIN, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::PinNotFound))?;
    let pin_str = keyring
        .get(&key)
        .cloned()
        .ok_or(AkonError::Keyring(KeyringError::PinNotFound))?;
    // Mirror production retrieval behavior: enforce a 30-char internal limit
    let pin_trimmed = pin_str.trim().to_string();
    let stored = if pin_trimmed.chars().count() > 30 {
        pin_trimmed.chars().take(30).collect::<String>()
    } else {
        pin_trimmed.clone()
    };

    Ok(Pin::from_unchecked(stored))
}

/// Check if a PIN exists in the mock keyring for the given username
pub fn has_pin(username: &str) -> Result<bool, AkonError> {
    let key = make_key(KEYRING_SERVICE_PIN, username);
    let keyring = MOCK_KEYRING
        .lock()
        .map_err(|_| AkonError::Keyring(KeyringError::ServiceUnavailable))?;
    Ok(keyring.contains_key(&key))
}

/// Delete a PIN from the mock keyring
pub fn delete_pin(username: &str) -> Result<(), AkonError> {
    let key = make_key(KEYRING_SERVICE_PIN, username);
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

    #[test]
    fn test_long_pin_truncation_and_generate_password() {
    use crate::auth::password::generate_password;

        let username = "test_long_pin_user";

        // Clean up first
        let _ = delete_pin(username);
        let _ = delete_otp_secret(username);

        // Create a long PIN (>30 chars)
        let long_pin = "012345678901234567890123456789012345".to_string(); // 36 chars
        let pin = Pin::from_unchecked(long_pin.clone());

        // Store long PIN and a valid OTP secret
        store_pin(username, &pin).expect("Failed to store long PIN");
        store_otp_secret(username, "JBSWY3DPEHPK3PXP").expect("Failed to store OTP secret");

        // Now generate password using generate_password which should retrieve and truncate
        let result = generate_password(username);
        assert!(result.is_ok(), "generate_password failed: {:?}", result.err());

        let password = result.unwrap();
        let pwd_str = password.expose();

        // The stored PIN should be silently truncated to 30 chars
        let expected_pin_prefix = long_pin.chars().take(30).collect::<String>();
        assert!(
            pwd_str.starts_with(&expected_pin_prefix),
            "Password does not start with truncated PIN: {} vs {}",
            pwd_str,
            expected_pin_prefix
        );

        // OTP part should be 6 digits at the end
        assert!(pwd_str.len() >= 6);
        let otp_part = &pwd_str[pwd_str.len() - 6..];
        assert!(otp_part.chars().all(|c| c.is_ascii_digit()));

        // Clean up
        delete_pin(username).expect("Failed to delete PIN");
        delete_otp_secret(username).expect("Failed to delete OTP");
    }
}
