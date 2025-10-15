//! Unit tests for type definitions and wrappers
//!
//! Tests ConnectionState, KeyringEntry, IpcMessage types, and secure wrappers.

use akon_core::types::{ConnectionState, IpcMessage, KeyringEntry, Pin, VpnPassword, TotpToken};
use akon_core::error::OtpError;
use std::time::SystemTime;

#[test]
fn test_connection_state_default() {
    let state = ConnectionState::default();
    assert_eq!(state, ConnectionState::Disconnected);
}

#[test]
fn test_connection_state_disconnected() {
    let state = ConnectionState::Disconnected;
    assert_eq!(state, ConnectionState::Disconnected);
}

#[test]
fn test_connection_state_connecting() {
    let state = ConnectionState::Connecting;
    assert_eq!(state, ConnectionState::Connecting);
}

#[test]
fn test_connection_state_connected() {
    let now = SystemTime::now();
    let state = ConnectionState::Connected {
        connected_at: now,
        server: "vpn.example.com".to_string(),
    };

    match state {
        ConnectionState::Connected { connected_at, server } => {
            assert_eq!(connected_at, now);
            assert_eq!(server, "vpn.example.com");
        }
        _ => panic!("Expected Connected state"),
    }
}

#[test]
fn test_connection_state_error() {
    let state = ConnectionState::Error {
        message: "Connection failed".to_string(),
    };

    match state {
        ConnectionState::Error { message } => {
            assert_eq!(message, "Connection failed");
        }
        _ => panic!("Expected Error state"),
    }
}

#[test]
fn test_connection_state_clone() {
    let state = ConnectionState::Connecting;
    let cloned = state.clone();
    assert_eq!(state, cloned);
}

#[test]
fn test_keyring_entry_creation() {
    let now = SystemTime::now();
    let entry = KeyringEntry {
        service: "akon-vpn-otp".to_string(),
        username: "testuser".to_string(),
        created: now,
    };

    assert_eq!(entry.service, "akon-vpn-otp");
    assert_eq!(entry.username, "testuser");
    assert_eq!(entry.created, now);
}

#[test]
fn test_keyring_entry_clone() {
    let now = SystemTime::now();
    let entry = KeyringEntry {
        service: "akon-vpn-otp".to_string(),
        username: "testuser".to_string(),
        created: now,
    };

    let cloned = entry.clone();
    assert_eq!(entry, cloned);
}

#[test]
fn test_ipc_message_status_request() {
    let msg = IpcMessage::StatusRequest;
    assert_eq!(msg, IpcMessage::StatusRequest);
}

#[test]
fn test_ipc_message_status_response() {
    let state = ConnectionState::Disconnected;
    let msg = IpcMessage::StatusResponse(state.clone());

    match msg {
        IpcMessage::StatusResponse(s) => assert_eq!(s, state),
        _ => panic!("Expected StatusResponse"),
    }
}

#[test]
fn test_ipc_message_connect_request() {
    let msg = IpcMessage::ConnectRequest {
        server: "vpn.example.com".to_string(),
        username: "testuser".to_string(),
    };

    match msg {
        IpcMessage::ConnectRequest { server, username } => {
            assert_eq!(server, "vpn.example.com");
            assert_eq!(username, "testuser");
        }
        _ => panic!("Expected ConnectRequest"),
    }
}

#[test]
fn test_ipc_message_connect_response_ok() {
    let msg = IpcMessage::ConnectResponse(Ok(()));

    match msg {
        IpcMessage::ConnectResponse(result) => assert!(result.is_ok()),
        _ => panic!("Expected ConnectResponse"),
    }
}

#[test]
fn test_ipc_message_connect_response_err() {
    let msg = IpcMessage::ConnectResponse(Err("Failed".to_string()));

    match msg {
        IpcMessage::ConnectResponse(result) => {
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), "Failed");
        }
        _ => panic!("Expected ConnectResponse"),
    }
}

#[test]
fn test_ipc_message_disconnect_request() {
    let msg = IpcMessage::DisconnectRequest;
    assert_eq!(msg, IpcMessage::DisconnectRequest);
}

#[test]
fn test_ipc_message_disconnect_response_ok() {
    let msg = IpcMessage::DisconnectResponse(Ok(()));

    match msg {
        IpcMessage::DisconnectResponse(result) => assert!(result.is_ok()),
        _ => panic!("Expected DisconnectResponse"),
    }
}

#[test]
fn test_ipc_message_disconnect_response_err() {
    let msg = IpcMessage::DisconnectResponse(Err("Failed".to_string()));

    match msg {
        IpcMessage::DisconnectResponse(result) => {
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), "Failed");
        }
        _ => panic!("Expected DisconnectResponse"),
    }
}

#[test]
fn test_ipc_message_shutdown() {
    let msg = IpcMessage::Shutdown;
    assert_eq!(msg, IpcMessage::Shutdown);
}

#[test]
fn test_ipc_message_clone() {
    let msg = IpcMessage::StatusRequest;
    let cloned = msg.clone();
    assert_eq!(msg, cloned);
}

#[test]
fn test_ipc_message_serialization() {
    // Test that messages can be serialized/deserialized
    let msg = IpcMessage::ConnectRequest {
        server: "vpn.example.com".to_string(),
        username: "testuser".to_string(),
    };

    let serialized = serde_json::to_string(&msg).expect("Failed to serialize");
    let deserialized: IpcMessage = serde_json::from_str(&serialized).expect("Failed to deserialize");

    assert_eq!(msg, deserialized);
}

#[test]
fn test_connection_state_serialization() {
    let state = ConnectionState::Connecting;

    let serialized = serde_json::to_string(&state).expect("Failed to serialize");
    let deserialized: ConnectionState = serde_json::from_str(&serialized).expect("Failed to deserialize");

    assert_eq!(state, deserialized);
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn test_pin_valid_four_digits() {
        let pin = Pin::new("1234".to_string());
        assert!(pin.is_ok());
        assert_eq!(pin.unwrap().expose(), "1234");
    }

    #[test]
    fn test_pin_valid_all_zeros() {
        let pin = Pin::new("0000".to_string());
        assert!(pin.is_ok());
        assert_eq!(pin.unwrap().expose(), "0000");
    }

    #[test]
    fn test_pin_valid_all_nines() {
        let pin = Pin::new("9999".to_string());
        assert!(pin.is_ok());
        assert_eq!(pin.unwrap().expose(), "9999");
    }

    #[test]
    fn test_pin_invalid_too_short() {
        let pin = Pin::new("123".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }

    #[test]
    fn test_pin_invalid_too_long() {
        let pin = Pin::new("12345".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }

    #[test]
    fn test_pin_invalid_contains_letters() {
        let pin = Pin::new("12ab".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }

    #[test]
    fn test_pin_invalid_contains_special_chars() {
        let pin = Pin::new("12@4".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }

    #[test]
    fn test_pin_invalid_contains_space() {
        let pin = Pin::new("12 4".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }

    #[test]
    fn test_pin_invalid_empty() {
        let pin = Pin::new("".to_string());
        assert!(pin.is_err());
        assert_eq!(pin.unwrap_err(), OtpError::InvalidPinFormat);
    }
}

#[cfg(test)]
mod vpn_password_tests {
    use super::*;

    #[test]
    fn test_vpn_password_from_components() {
        let pin = Pin::new("1234".to_string()).unwrap();
        let otp = TotpToken::new("567890".to_string());
        let password = VpnPassword::from_components(&pin, &otp);

        assert_eq!(password.expose(), "1234567890");
        assert_eq!(password.expose().len(), 10);
    }

    #[test]
    fn test_vpn_password_correct_format() {
        let pin = Pin::new("0000".to_string()).unwrap();
        let otp = TotpToken::new("123456".to_string());
        let password = VpnPassword::from_components(&pin, &otp);

        // Verify format: 4 digits (PIN) + 6 digits (OTP) = 10 characters
        let pwd_str = password.expose();
        assert_eq!(pwd_str.len(), 10);
        assert!(pwd_str.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_vpn_password_new() {
        let password = VpnPassword::new("1234567890".to_string());
        assert_eq!(password.expose(), "1234567890");
    }
}
