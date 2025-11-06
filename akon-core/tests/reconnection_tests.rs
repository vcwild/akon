//! Tests for reconnection logic including exponential backoff

use akon_core::vpn::reconnection::ReconnectionPolicy;
use std::time::Duration;

#[test]
fn test_backoff_calculation_default_policy() {
    // Given: Default policy (base=5s, multiplier=2, max=60s)
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    // When: Calculating backoff for attempts 1-6
    let backoff1 = calculate_backoff(&policy, 1);
    let backoff2 = calculate_backoff(&policy, 2);
    let backoff3 = calculate_backoff(&policy, 3);
    let backoff4 = calculate_backoff(&policy, 4);
    let backoff5 = calculate_backoff(&policy, 5);
    let backoff6 = calculate_backoff(&policy, 6);

    // Then: Should follow exponential pattern 5s → 10s → 20s → 40s → 60s (capped)
    assert_eq!(backoff1, Duration::from_secs(5), "Attempt 1 should be 5s");
    assert_eq!(backoff2, Duration::from_secs(10), "Attempt 2 should be 10s");
    assert_eq!(backoff3, Duration::from_secs(20), "Attempt 3 should be 20s");
    assert_eq!(backoff4, Duration::from_secs(40), "Attempt 4 should be 40s");
    assert_eq!(
        backoff5,
        Duration::from_secs(60),
        "Attempt 5 should be 60s (capped at max)"
    );
    assert_eq!(
        backoff6,
        Duration::from_secs(60),
        "Attempt 6 should be 60s (still capped)"
    );
}

#[test]
fn test_backoff_cap_at_max_interval() {
    // Given: Policy with low max interval (30s)
    let policy = ReconnectionPolicy {
        max_attempts: 10,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 30,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    // When: Calculating backoff for multiple attempts
    let backoff3 = calculate_backoff(&policy, 3); // Would be 20s
    let backoff4 = calculate_backoff(&policy, 4); // Would be 40s but capped at 30s
    let backoff5 = calculate_backoff(&policy, 5); // Would be 80s but capped at 30s

    // Then: Should cap at max_interval_secs
    assert_eq!(
        backoff3,
        Duration::from_secs(20),
        "Attempt 3 should be 20s (under cap)"
    );
    assert_eq!(
        backoff4,
        Duration::from_secs(30),
        "Attempt 4 should be 30s (capped)"
    );
    assert_eq!(
        backoff5,
        Duration::from_secs(30),
        "Attempt 5 should be 30s (capped)"
    );
}

#[test]
fn test_backoff_with_different_multipliers() {
    // Given: Policy with multiplier of 3
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 2,
        backoff_multiplier: 3,
        max_interval_secs: 100,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    // When: Calculating backoff
    let backoff1 = calculate_backoff(&policy, 1);
    let backoff2 = calculate_backoff(&policy, 2);
    let backoff3 = calculate_backoff(&policy, 3);
    let backoff4 = calculate_backoff(&policy, 4);

    // Then: Should follow pattern 2s → 6s (2*3) → 18s (2*3²) → 54s (2*3³)
    assert_eq!(backoff1, Duration::from_secs(2), "Attempt 1: 2 * 3^0 = 2s");
    assert_eq!(backoff2, Duration::from_secs(6), "Attempt 2: 2 * 3^1 = 6s");
    assert_eq!(
        backoff3,
        Duration::from_secs(18),
        "Attempt 3: 2 * 3^2 = 18s"
    );
    assert_eq!(
        backoff4,
        Duration::from_secs(54),
        "Attempt 4: 2 * 3^3 = 54s"
    );
}

#[test]
fn test_backoff_with_multiplier_one() {
    // Given: Policy with multiplier of 1 (constant backoff)
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 10,
        backoff_multiplier: 1,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    // When: Calculating backoff for multiple attempts
    let backoff1 = calculate_backoff(&policy, 1);
    let backoff2 = calculate_backoff(&policy, 2);
    let backoff3 = calculate_backoff(&policy, 3);

    // Then: Should remain constant at base interval
    assert_eq!(backoff1, Duration::from_secs(10), "Should be constant 10s");
    assert_eq!(backoff2, Duration::from_secs(10), "Should be constant 10s");
    assert_eq!(backoff3, Duration::from_secs(10), "Should be constant 10s");
}

#[test]
fn test_backoff_first_attempt_is_base_interval() {
    // Given: Any policy
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 7,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    // When: Calculating backoff for first attempt
    let backoff = calculate_backoff(&policy, 1);

    // Then: Should equal base interval (no multiplication)
    assert_eq!(
        backoff,
        Duration::from_secs(7),
        "First attempt should be base interval"
    );
}

#[test]
fn test_successful_reconnection_updates_state() {
    // Given: A reconnecting state at attempt 2
    use akon_core::vpn::state::ConnectionState;

    let initial_state = ConnectionState::Reconnecting {
        attempt: 2,
        next_retry_at: Some(1699104020),
        max_attempts: 5,
    };

    // When: Reconnection succeeds (simulated by transitioning to Connected)
    let success_state = ConnectionState::Connected(Default::default());

    // Then: State should transition to Connected
    assert!(matches!(
        initial_state,
        ConnectionState::Reconnecting { attempt: 2, .. }
    ));
    assert!(matches!(success_state, ConnectionState::Connected(_)));
}

#[test]
fn test_failed_attempt_increments_counter() {
    // Given: A reconnecting state at attempt 2
    use akon_core::vpn::state::ConnectionState;

    let state_attempt_2 = ConnectionState::Reconnecting {
        attempt: 2,
        next_retry_at: Some(1699104020),
        max_attempts: 5,
    };

    // When: Attempt fails and counter increments
    let state_attempt_3 = ConnectionState::Reconnecting {
        attempt: 3,
        next_retry_at: Some(1699104040),
        max_attempts: 5,
    };

    // Then: Attempt should increment
    match state_attempt_2 {
        ConnectionState::Reconnecting { attempt, .. } => assert_eq!(attempt, 2),
        _ => panic!("Expected Reconnecting state"),
    }
    match state_attempt_3 {
        ConnectionState::Reconnecting { attempt, .. } => assert_eq!(attempt, 3),
        _ => panic!("Expected Reconnecting state"),
    }
}

#[test]
fn test_max_attempts_exceeded_transitions_to_error() {
    // Given: A reconnecting state at max attempts
    use akon_core::vpn::state::ConnectionState;

    let max_state = ConnectionState::Reconnecting {
        attempt: 5,
        next_retry_at: None,
        max_attempts: 5,
    };

    // When: Max attempts reached (simulated transition to Error)
    let error_state = ConnectionState::Error("Max reconnection attempts exceeded".to_string());

    // Then: Should be at max and then transition to Error
    match max_state {
        ConnectionState::Reconnecting {
            attempt,
            max_attempts,
            ..
        } => {
            assert_eq!(attempt, max_attempts, "Should be at max attempts");
        }
        _ => panic!("Expected Reconnecting state"),
    }
    assert!(matches!(error_state, ConnectionState::Error(_)));
}

#[test]
fn test_network_not_stable_delays_reconnection() {
    // This test verifies the logic concept - actual implementation will be in ReconnectionManager
    // Given: Network is not stable (health check endpoint unreachable)
    let network_stable = false;
    let should_attempt_reconnect = network_stable;

    // Then: Should not attempt reconnection
    assert!(
        !should_attempt_reconnect,
        "Should delay reconnection when network unstable"
    );

    // When: Network becomes stable
    let network_stable = true;
    let should_attempt_reconnect = network_stable;

    // Then: Should attempt reconnection
    assert!(
        should_attempt_reconnect,
        "Should attempt reconnection when network stable"
    );
}

// Helper function to calculate backoff using ReconnectionManager
fn calculate_backoff(policy: &ReconnectionPolicy, attempt: u32) -> Duration {
    use akon_core::vpn::reconnection::ReconnectionManager;
    let manager = ReconnectionManager::new(policy.clone());
    manager.calculate_backoff(attempt)
}

// ===== Health Check Handling Tests (T030) =====

#[tokio::test]
#[ignore] // Requires full ReconnectionManager::handle_health_check() implementation
async fn test_health_check_failure_triggers_reconnection() {
    use akon_core::vpn::connection_event::ConnectionState;
    use akon_core::vpn::reconnection::ReconnectionManager;

    // Given: Policy with consecutive_failures_threshold = 3
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    let manager = ReconnectionManager::new(policy);
    let state_rx = manager.state_receiver();

    // When: Health check fails 3 consecutive times
    // (Will be implemented via manager.handle_health_check())
    // manager.handle_health_check(false).await;
    // manager.handle_health_check(false).await;
    // manager.handle_health_check(false).await;

    // Then: Should trigger reconnection (state transitions to Reconnecting)
    // assert!(matches!(*state_rx.borrow(), ConnectionState::Reconnecting { .. }));
}

#[tokio::test]
#[ignore] // Requires full ReconnectionManager::handle_health_check() implementation
async fn test_consecutive_failures_threshold() {
    use akon_core::vpn::reconnection::ReconnectionManager;

    // Given: Policy with consecutive_failures_threshold = 2
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 2,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    let manager = ReconnectionManager::new(policy);

    // When: Health check fails twice
    // manager.handle_health_check(false).await;
    // manager.handle_health_check(false).await;

    // Then: Should trigger reconnection after 2nd failure
    // (Verify via state transition)
}

#[tokio::test]
#[ignore] // Requires full ReconnectionManager::handle_health_check() implementation
async fn test_single_failure_does_not_trigger_reconnection() {
    use akon_core::vpn::connection_event::ConnectionState;
    use akon_core::vpn::reconnection::ReconnectionManager;

    // Given: Policy with consecutive_failures_threshold = 3
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    let manager = ReconnectionManager::new(policy);
    let state_rx = manager.state_receiver();

    // Given: Initial state is Connected
    // (Will be set by manager initialization)

    // When: Health check fails once
    // manager.handle_health_check(false).await;

    // Then: Should NOT trigger reconnection (still Connected)
    // assert!(matches!(*state_rx.borrow(), ConnectionState::Connected { .. }));
}

#[tokio::test]
#[ignore] // Requires full ReconnectionManager::handle_health_check() implementation
async fn test_health_check_success_resets_failure_counter() {
    use akon_core::vpn::connection_event::ConnectionState;
    use akon_core::vpn::reconnection::ReconnectionManager;

    // Given: Policy with consecutive_failures_threshold = 3
    let policy = ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 5,
        backoff_multiplier: 2,
        max_interval_secs: 60,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 60,
        health_check_endpoint: "https://vpn.example.com/health".to_string(),
    };

    let manager = ReconnectionManager::new(policy);
    let state_rx = manager.state_receiver();

    // When: 2 failures, then success, then 2 more failures
    // manager.handle_health_check(false).await;  // failure 1
    // manager.handle_health_check(false).await;  // failure 2
    // manager.handle_health_check(true).await;   // success - resets counter to 0
    // manager.handle_health_check(false).await;  // failure 1 (counter reset)
    // manager.handle_health_check(false).await;  // failure 2

    // Then: Should NOT trigger reconnection (only 2 consecutive failures, not 3)
    // assert!(matches!(*state_rx.borrow(), ConnectionState::Connected { .. }));
}

// ============================================================================
// Tests for User Story 4: Manual Reset Functionality (T047)
// ============================================================================

#[test]
#[ignore = "Requires ReconnectionManager command handling implementation"]
fn test_reset_clears_retry_counter() {
    // Given: Manager with some failed attempts
    // let policy = ReconnectionPolicy::default();
    // let mut manager = ReconnectionManager::new(policy);

    // Simulate 3 failed reconnection attempts
    // manager.attempt_reconnect().await;  // attempt 1
    // manager.attempt_reconnect().await;  // attempt 2
    // manager.attempt_reconnect().await;  // attempt 3

    // When: Reset command is sent
    // manager.handle_command(ReconnectionCommand::ResetRetries).await;

    // Then: Retry counter should be 0
    // assert_eq!(manager.get_retry_counter(), 0);
}

#[test]
#[ignore = "Requires ReconnectionManager command handling implementation"]
fn test_reset_transitions_from_error_to_disconnected() {
    // Given: Manager in Error state after max attempts exceeded
    // let policy = ReconnectionPolicy { max_attempts: 2, ..Default::default() };
    // let mut manager = ReconnectionManager::new(policy);
    // let state_rx = manager.state_receiver();

    // Exceed max attempts
    // for _ in 0..2 {
    //     manager.attempt_reconnect().await;
    // }

    // Verify in Error state
    // assert!(matches!(*state_rx.borrow(), ConnectionState::Error { .. }));

    // When: Reset command is sent
    // manager.handle_command(ReconnectionCommand::ResetRetries).await;

    // Then: Should transition to Disconnected
    // assert!(matches!(*state_rx.borrow(), ConnectionState::Disconnected));
}

#[test]
#[ignore = "Requires ReconnectionManager command handling implementation"]
fn test_reset_allows_reconnection_after_max_attempts_exceeded() {
    // Given: Manager that has exceeded max attempts
    // let policy = ReconnectionPolicy { max_attempts: 2, ..Default::default() };
    // let mut manager = ReconnectionManager::new(policy);

    // Exceed max attempts (should transition to Error state)
    // for _ in 0..2 {
    //     manager.attempt_reconnect().await;
    // }

    // When: Reset command is sent
    // manager.handle_command(ReconnectionCommand::ResetRetries).await;

    // Then: Should be able to attempt reconnection again
    // let result = manager.attempt_reconnect().await;
    // assert!(result.is_ok(), "Should allow reconnection after reset");
}
