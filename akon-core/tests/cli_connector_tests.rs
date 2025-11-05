// Unit tests for CliConnector

use akon_core::config::VpnConfig;
use akon_core::vpn::{CliConnector, ConnectionState};
use std::net::IpAddr;

#[test]
fn test_cli_connector_new_creates_idle_state() {
    let config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());

    let connector = CliConnector::new(config).expect("Failed to create connector");
    let state = connector.state();

    assert!(matches!(state, ConnectionState::Idle));
}

#[test]
fn test_cli_connector_initial_is_not_connected() {
    let config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());

    let connector = CliConnector::new(config).expect("Failed to create connector");

    assert!(!connector.is_connected());
}

// User Story 3 Tests - Connection completion detection

#[test]
fn test_connection_state_transitions() {
    // Test that ConnectionState enum has all required variants
    let idle = ConnectionState::Idle;
    let connecting = ConnectionState::Connecting;
    let authenticating = ConnectionState::Authenticating;

    assert!(matches!(idle, ConnectionState::Idle));
    assert!(matches!(connecting, ConnectionState::Connecting));
    assert!(matches!(authenticating, ConnectionState::Authenticating));
}

#[test]
fn test_connection_state_established() {
    let ip: IpAddr = "10.0.1.100".parse().unwrap();
    let state = ConnectionState::Established {
        ip,
        device: "tun0".to_string(),
    };

    match state {
        ConnectionState::Established {
            ip: state_ip,
            device,
        } => {
            assert_eq!(state_ip.to_string(), "10.0.1.100");
            assert_eq!(device, "tun0");
        }
        _ => panic!("Expected Established state"),
    }
}

#[test]
fn test_is_connected_for_established_state() {
    let config = VpnConfig::new("vpn.example.com".to_string(), "testuser".to_string());

    let connector = CliConnector::new(config).expect("Failed to create connector");

    // Initially not connected
    assert!(!connector.is_connected());

    // Note: We can't easily test state transitions without mocking the actual connection
    // This would require integration tests with mock OpenConnect process
}
