// Unit tests for ConnectionEvent enum and related types

use akon_core::vpn::{ConnectionEvent, DisconnectReason};
use std::net::IpAddr;

#[test]
fn test_connection_event_process_started() {
    let event = ConnectionEvent::ProcessStarted { pid: 1234 };
    assert!(matches!(
        event,
        ConnectionEvent::ProcessStarted { pid: 1234 }
    ));
}

#[test]
fn test_connection_event_connected_with_ip() {
    let ip: IpAddr = "10.0.1.100".parse().unwrap();
    let event = ConnectionEvent::Connected {
        ip,
        device: "tun0".to_string(),
    };

    match event {
        ConnectionEvent::Connected { ip: evt_ip, device } => {
            assert_eq!(evt_ip.to_string(), "10.0.1.100");
            assert_eq!(device, "tun0");
        }
        _ => panic!("Expected Connected event"),
    }
}

#[test]
fn test_connection_event_equality() {
    let ip: IpAddr = "10.0.1.100".parse().unwrap();
    let event1 = ConnectionEvent::Connected {
        ip,
        device: "tun0".to_string(),
    };
    let event2 = ConnectionEvent::Connected {
        ip,
        device: "tun0".to_string(),
    };

    assert_eq!(event1, event2);
}

#[test]
fn test_disconnect_reason_variants() {
    let reason = DisconnectReason::UserRequested;
    assert!(matches!(reason, DisconnectReason::UserRequested));

    let reason = DisconnectReason::Timeout;
    assert!(matches!(reason, DisconnectReason::Timeout));
}
