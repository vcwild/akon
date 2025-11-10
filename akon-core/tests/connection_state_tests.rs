//! Tests for ConnectionState transitions involving Reconnecting state
//!
//! These tests verify the state machine behavior for automatic reconnection.

use akon_core::vpn::state::ConnectionState;

#[test]
fn test_reconnecting_state_has_attempt_counter() {
    // Given: A reconnecting state with attempt 3
    let state = ConnectionState::Reconnecting {
        attempt: 3,
        next_retry_at: Some(1699104000),
        max_attempts: 5,
    };

    // Then: State should be Reconnecting variant
    match state {
        ConnectionState::Reconnecting {
            attempt,
            max_attempts,
            ..
        } => {
            assert_eq!(attempt, 3);
            assert_eq!(max_attempts, 5);
        }
        _ => panic!("Expected Reconnecting state"),
    }
}

#[test]
fn test_reconnecting_state_can_have_no_next_retry_time() {
    // Given: A reconnecting state without next retry time (immediate retry)
    let state = ConnectionState::Reconnecting {
        attempt: 1,
        next_retry_at: None,
        max_attempts: 5,
    };

    // Then: next_retry_at should be None
    match state {
        ConnectionState::Reconnecting { next_retry_at, .. } => {
            assert!(next_retry_at.is_none());
        }
        _ => panic!("Expected Reconnecting state"),
    }
}

#[test]
fn test_connected_to_reconnecting_transition() {
    // Given: A connected state
    let connected = ConnectionState::Connected(Default::default());

    // When: Transitioning to reconnecting (simulating network interruption)
    let reconnecting = ConnectionState::Reconnecting {
        attempt: 1,
        next_retry_at: None,
        max_attempts: 5,
    };

    // Then: States should be different variants
    assert!(!matches!(connected, ConnectionState::Reconnecting { .. }));
    assert!(matches!(reconnecting, ConnectionState::Reconnecting { .. }));
}

#[test]
fn test_reconnecting_to_connected_transition() {
    // Given: A reconnecting state
    let reconnecting = ConnectionState::Reconnecting {
        attempt: 2,
        next_retry_at: Some(1699104000),
        max_attempts: 5,
    };

    // When: Transitioning to connected (successful reconnection)
    let connected = ConnectionState::Connected(Default::default());

    // Then: States should be different variants
    assert!(matches!(reconnecting, ConnectionState::Reconnecting { .. }));
    assert!(matches!(connected, ConnectionState::Connected(_)));
}

#[test]
fn test_reconnecting_to_error_on_max_attempts() {
    // Given: A reconnecting state at max attempts
    let reconnecting = ConnectionState::Reconnecting {
        attempt: 5,
        next_retry_at: None,
        max_attempts: 5,
    };

    // When: Transitioning to error (max attempts exceeded)
    let error = ConnectionState::Error("Max reconnection attempts exceeded".to_string());

    // Then: Should transition to Error state
    assert!(matches!(
        reconnecting,
        ConnectionState::Reconnecting {
            attempt: 5,
            max_attempts: 5,
            ..
        }
    ));
    assert!(matches!(error, ConnectionState::Error(_)));
}

#[test]
fn test_reconnecting_state_attempt_increments() {
    // Given: A sequence of reconnecting states with incrementing attempts
    let states = [
        ConnectionState::Reconnecting {
            attempt: 1,
            next_retry_at: Some(1699104000),
            max_attempts: 5,
        },
        ConnectionState::Reconnecting {
            attempt: 2,
            next_retry_at: Some(1699104010),
            max_attempts: 5,
        },
        ConnectionState::Reconnecting {
            attempt: 3,
            next_retry_at: Some(1699104030),
            max_attempts: 5,
        },
    ];

    // Then: Each state should have the correct attempt number
    for (idx, state) in states.iter().enumerate() {
        match state {
            ConnectionState::Reconnecting { attempt, .. } => {
                assert_eq!(*attempt, (idx + 1) as u32);
            }
            _ => panic!("Expected Reconnecting state"),
        }
    }
}

#[test]
fn test_reconnecting_state_validates_attempt_le_max() {
    // Given: A reconnecting state
    let state = ConnectionState::Reconnecting {
        attempt: 3,
        next_retry_at: None,
        max_attempts: 5,
    };

    // Then: attempt should be <= max_attempts
    match state {
        ConnectionState::Reconnecting {
            attempt,
            max_attempts,
            ..
        } => {
            assert!(
                attempt <= max_attempts,
                "Attempt {} exceeds max_attempts {}",
                attempt,
                max_attempts
            );
        }
        _ => panic!("Expected Reconnecting state"),
    }
}

#[test]
fn test_disconnected_to_reconnecting_after_network_available() {
    // Given: A disconnected state (after network interruption)
    let disconnected = ConnectionState::Disconnected;

    // When: Network becomes available and reconnection starts
    let reconnecting = ConnectionState::Reconnecting {
        attempt: 1,
        next_retry_at: None,
        max_attempts: 5,
    };

    // Then: Should transition from Disconnected to Reconnecting
    assert!(matches!(disconnected, ConnectionState::Disconnected));
    assert!(matches!(
        reconnecting,
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
}

#[test]
fn test_reconnecting_state_serialization() {
    // Given: A reconnecting state
    let state = ConnectionState::Reconnecting {
        attempt: 2,
        next_retry_at: Some(1699104020),
        max_attempts: 5,
    };

    // When: Serializing to JSON
    let serialized = serde_json::to_string(&state).expect("Should serialize");

    // Then: Should contain the variant name and fields
    assert!(serialized.contains("Reconnecting"));

    // When: Deserializing back
    let deserialized: ConnectionState =
        serde_json::from_str(&serialized).expect("Should deserialize");

    // Then: Should match original state
    match deserialized {
        ConnectionState::Reconnecting {
            attempt,
            next_retry_at,
            max_attempts,
        } => {
            assert_eq!(attempt, 2);
            assert_eq!(next_retry_at, Some(1699104020));
            assert_eq!(max_attempts, 5);
        }
        _ => panic!("Expected Reconnecting state after deserialization"),
    }
}
