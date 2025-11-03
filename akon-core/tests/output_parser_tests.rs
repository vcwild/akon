// Unit tests for OutputParser

use akon_core::vpn::{ConnectionEvent, OutputParser};

#[test]
fn test_parse_tun_configured() {
    let parser = OutputParser::new();
    let line = "Connected tun0 as 10.0.1.100";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::TunConfigured { device, ip } => {
            assert_eq!(device, "tun0");
            assert_eq!(ip.to_string(), "10.0.1.100");
        }
        _ => panic!("Expected TunConfigured event, got {:?}", event),
    }
}

#[test]
fn test_parse_established_connection() {
    let parser = OutputParser::new();
    let line = "Established connection";
    let event = parser.parse_line(line);

    // Should return Authenticating or appropriate event
    assert!(
        matches!(event, ConnectionEvent::Authenticating { .. })
            || matches!(event, ConnectionEvent::Connected { .. })
            || matches!(event, ConnectionEvent::F5SessionEstablished { .. })
    );
}

#[test]
fn test_parse_authentication_failed() {
    let parser = OutputParser::new();
    let line = "Failed to authenticate";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::Error { kind, .. } => {
            // Should be AuthenticationFailed error
            assert!(kind.to_string().contains("Authentication"));
        }
        _ => panic!("Expected Error event, got {:?}", event),
    }
}

#[test]
fn test_parse_unknown_output() {
    let parser = OutputParser::new();
    let line = "This is some random unknown output";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::UnknownOutput { line: output } => {
            assert_eq!(output, line);
        }
        _ => panic!("Expected UnknownOutput event, got {:?}", event),
    }
}

// User Story 2 Tests - Enhanced progress tracking

#[test]
fn test_parse_post_authentication() {
    let parser = OutputParser::new();
    let line = "POST https://vpn.example.com/";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::Authenticating { message } => {
            assert!(message.contains("Authenticating") || message.contains("server"));
        }
        _ => panic!("Expected Authenticating event for POST, got {:?}", event),
    }
}

#[test]
fn test_parse_connect_response() {
    let parser = OutputParser::new();
    let line = "Got CONNECT response: HTTP/1.1 200 OK";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::Authenticating { message } => {
            assert!(message.contains("response") || message.contains("server"));
        }
        _ => panic!("Expected Authenticating event for CONNECT response, got {:?}", event),
    }
}

#[test]
fn test_parse_f5_session_established() {
    let parser = OutputParser::new();
    let line = "Connected to F5 Session Manager";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::F5SessionEstablished { .. } => {
            // Success
        }
        _ => panic!("Expected F5SessionEstablished event, got {:?}", event),
    }
}

#[test]
fn test_parse_ipv4_extraction() {
    let parser = OutputParser::new();

    // Test various IPv4 formats
    let lines = vec![
        "Connected tun0 as 10.0.1.100",
        "Connected tun1 as 192.168.1.50",
        "Connected tun2 as 172.16.0.1",
    ];

    for line in lines {
        let event = parser.parse_line(line);
        match event {
            ConnectionEvent::TunConfigured { ip, .. } => {
                assert!(ip.is_ipv4(), "Expected IPv4 address in line: {}", line);
            }
            _ => panic!("Expected TunConfigured for line: {}", line),
        }
    }
}

#[test]
fn test_parse_ipv6_extraction() {
    let parser = OutputParser::new();
    let line = "Connected tun0 as 2001:db8::1";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::TunConfigured { ip, device } => {
            assert!(ip.is_ipv6());
            assert_eq!(device, "tun0");
        }
        _ => panic!("Expected TunConfigured event with IPv6, got {:?}", event),
    }
}
