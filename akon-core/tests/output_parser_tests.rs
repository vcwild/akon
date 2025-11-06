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
        _ => panic!(
            "Expected Authenticating event for CONNECT response, got {:?}",
            event
        ),
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

// User Story 6 Tests - Enhanced error diagnostics

#[test]
fn test_parse_ssl_error() {
    let parser = OutputParser::new();

    let test_cases = vec![
        "SSL connection failure detected",
        "TLS handshake failed",
        "SSL: certificate verify failed",
        "connection failure: TLS error",
    ];

    for line in test_cases {
        let event = parser.parse_error(line);
        match event {
            ConnectionEvent::Error { kind, raw_output } => {
                assert!(
                    kind.to_string().contains("SSL")
                        || kind.to_string().contains("TLS")
                        || kind.to_string().contains("Network"),
                    "Expected SSL/TLS error for line: {}",
                    line
                );
                assert_eq!(raw_output, line);
            }
            _ => panic!("Expected Error event for SSL error, got {:?}", event),
        }
    }
}

#[test]
fn test_parse_certificate_error() {
    let parser = OutputParser::new();

    let test_cases = vec![
        "certificate verification failed",
        "cert is invalid",
        "Certificate validation error",
    ];

    for line in test_cases {
        let event = parser.parse_error(line);
        match event {
            ConnectionEvent::Error { kind, raw_output } => {
                assert!(
                    kind.to_string().contains("Certificate")
                        || kind.to_string().contains("Network"),
                    "Expected certificate error for line: {}",
                    line
                );
                assert_eq!(raw_output, line);
            }
            _ => panic!(
                "Expected Error event for certificate error, got {:?}",
                event
            ),
        }
    }
}

#[test]
fn test_parse_tun_device_error() {
    let parser = OutputParser::new();

    let test_cases = vec![
        "failed to open tun device",
        "tun0 error: permission denied",
        "no tun device available",
    ];

    for line in test_cases {
        let event = parser.parse_error(line);
        match event {
            ConnectionEvent::Error { kind, raw_output } => {
                assert!(
                    kind.to_string().contains("TUN")
                        || kind.to_string().contains("sudo")
                        || kind.to_string().contains("Failed"),
                    "Expected TUN device error for line: {}",
                    line
                );
                assert_eq!(raw_output, line);
            }
            _ => panic!("Expected Error event for TUN device error, got {:?}", event),
        }
    }
}

#[test]
fn test_parse_dns_error() {
    let parser = OutputParser::new();

    let test_cases = vec![
        "cannot resolve hostname vpn.example.com",
        "unknown host: vpn.example.com",
        "name resolution failed",
    ];

    for line in test_cases {
        let event = parser.parse_error(line);
        match event {
            ConnectionEvent::Error { kind, raw_output } => {
                assert!(
                    kind.to_string().contains("DNS")
                        || kind.to_string().contains("Network")
                        || kind.to_string().contains("resolution"),
                    "Expected DNS error for line: {}",
                    line
                );
                assert_eq!(raw_output, line);
            }
            _ => panic!("Expected Error event for DNS error, got {:?}", event),
        }
    }
}

#[test]
fn test_parse_auth_error_still_works() {
    let parser = OutputParser::new();
    let line = "Failed to authenticate";
    let event = parser.parse_error(line);

    match event {
        ConnectionEvent::Error { kind, .. } => {
            assert!(
                kind.to_string().contains("Authentication"),
                "Expected authentication error, got: {}",
                kind
            );
        }
        _ => panic!("Expected Error event for auth failure, got {:?}", event),
    }
}
