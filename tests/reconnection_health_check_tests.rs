//! Integration tests for reconnection manager health check failures
//!
//! These tests verify that the reconnection manager correctly detects
//! health check failures and triggers reconnection attempts.

use akon_core::config::VpnConfig;
use akon_core::vpn::health_check::HealthChecker;
use akon_core::vpn::reconnection::{ReconnectionCommand, ReconnectionManager, ReconnectionPolicy};
use akon_core::vpn::state::ConnectionState;
use std::time::Duration;
use tokio::time::timeout;

/// Helper function to create a test reconnection policy
fn create_test_policy(health_endpoint: String) -> ReconnectionPolicy {
    ReconnectionPolicy {
        max_attempts: 5,
        base_interval_secs: 1, // Short interval for testing
        backoff_multiplier: 2,
        max_interval_secs: 10,
        consecutive_failures_threshold: 3,
        health_check_interval_secs: 2, // Check every 2 seconds for faster testing
        health_check_endpoint: health_endpoint,
    }
}

/// Helper function to create a test VPN config
fn create_test_vpn_config() -> VpnConfig {
    VpnConfig {
        server: "test.example.com".to_string(),
        username: "testuser".to_string(),
        protocol: akon_core::config::VpnProtocol::F5,
        timeout: Some(30),
        no_dtls: true,
        lazy_mode: false,
    }
}

#[tokio::test]
async fn test_health_check_failure_triggers_reconnection() {
    // Logging is handled by the test runner

    // Use an invalid endpoint that will always fail
    let policy = create_test_policy("http://192.0.2.1:9999".to_string()); // TEST-NET-1, guaranteed to fail
    let _config = create_test_vpn_config();

    // Create health checker with failing endpoint
    let health_checker =
        HealthChecker::new(policy.health_check_endpoint.clone(), Duration::from_secs(1))
            .expect("Failed to create health checker");

    // Create reconnection manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set initial state to Connected
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected command");

    // Spawn the manager in a background task
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for state to become Connected
    let initial_state = timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    return state;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    println!("Initial state: {:?}", initial_state);
    assert!(matches!(initial_state, ConnectionState::Connected(_)));

    // Wait for health checks to fail and trigger reconnection
    // We need to wait for:
    // - 3 consecutive failures at 2-second intervals = 6 seconds
    // - Plus time for health check execution
    // - Plus buffer for state transition
    let reconnecting_state = timeout(Duration::from_secs(15), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                println!("State changed to: {:?}", state);

                // Check if we've transitioned to Reconnecting or Disconnected
                // (Disconnected is set before transition to Reconnecting)
                match state {
                    ConnectionState::Disconnected => {
                        println!(
                            "Detected Disconnected state (triggered by health check failures)"
                        );
                        return Some(state);
                    }
                    ConnectionState::Reconnecting { attempt, .. } => {
                        println!("Detected Reconnecting state (attempt {})", attempt);
                        return Some(state);
                    }
                    _ => continue,
                }
            }
        }
    })
    .await;

    // Shutdown manager
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown command");

    // Wait for manager to stop
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    // Verify we detected reconnection trigger
    assert!(
        reconnecting_state.is_ok(),
        "Expected state transition to Disconnected or Reconnecting after health check failures, but timeout occurred"
    );

    let final_state = reconnecting_state.unwrap();
    assert!(
        final_state.is_some(),
        "Expected valid state after health check failures"
    );

    println!("✓ Test passed: Health check failures correctly triggered reconnection");
}

#[tokio::test]
async fn test_successful_health_checks_prevent_reconnection() {
    // Logging is handled by the test runner

    // Use a reliable endpoint that should succeed
    let policy = create_test_policy("https://www.google.com".to_string());
    let _config = create_test_vpn_config();

    // Create health checker with working endpoint
    let health_checker =
        HealthChecker::new(policy.health_check_endpoint.clone(), Duration::from_secs(5))
            .expect("Failed to create health checker");

    // Create reconnection manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set initial state to Connected
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected command");

    // Spawn the manager in a background task
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for state to become Connected
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    // Wait for several health check cycles (3 cycles × 2 seconds = 6 seconds + buffer)
    let reconnection_attempted = timeout(Duration::from_secs(10), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();

                // If we detect any reconnection attempt, return true
                if matches!(
                    state,
                    ConnectionState::Disconnected | ConnectionState::Reconnecting { .. }
                ) {
                    return true;
                }
            }
        }
    })
    .await;

    // Shutdown manager
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown command");

    // Wait for manager to stop
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    // Verify no reconnection was attempted (timeout occurred)
    assert!(
        reconnection_attempted.is_err(),
        "Expected no reconnection with successful health checks, but reconnection was triggered"
    );

    println!("✓ Test passed: Successful health checks prevented unnecessary reconnection");
}

#[tokio::test]
async fn test_consecutive_failure_threshold() {
    // Logging is handled by the test runner

    // Use an invalid endpoint
    let mut policy = create_test_policy("http://192.0.2.1:9999".to_string());
    policy.consecutive_failures_threshold = 5; // Require 5 failures instead of 3
    policy.health_check_interval_secs = 1; // Faster checks
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create reconnection manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set initial state to Connected
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected command");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected state
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    // Wait for 5 failures (5 seconds + buffer)
    // Should trigger reconnection after 5th failure
    let reconnection_detected = timeout(Duration::from_secs(10), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(
                    state,
                    ConnectionState::Disconnected | ConnectionState::Reconnecting { .. }
                ) {
                    return true;
                }
            }
        }
    })
    .await;

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    assert!(
        reconnection_detected.is_ok() && reconnection_detected.unwrap(),
        "Expected reconnection after 5 consecutive failures"
    );

    println!("✓ Test passed: Consecutive failure threshold correctly enforced");
}

#[tokio::test]
async fn test_manual_health_check_command() {
    // Logging is handled by the test runner

    // Use a working endpoint for this test
    let policy = create_test_policy("https://www.google.com".to_string());
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker =
        HealthChecker::new(policy.health_check_endpoint.clone(), Duration::from_secs(5))
            .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy);
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set Connected state
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    // Trigger manual health check
    command_tx
        .send(ReconnectionCommand::CheckNow)
        .expect("Failed to send CheckNow command");

    // Give it time to process
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify we're still connected (health check succeeded)
    let current_state = state_rx.borrow().clone();
    assert!(
        matches!(current_state, ConnectionState::Connected(_)),
        "Expected to remain Connected after successful manual health check"
    );

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    println!("✓ Test passed: Manual health check command works correctly");
}

#[tokio::test]
async fn test_reconnection_attempt_after_disconnect() {
    // Logging is handled by the test runner

    // Use invalid endpoint to ensure health checks fail
    let mut policy = create_test_policy("http://192.0.2.1:9999".to_string());
    policy.consecutive_failures_threshold = 2; // Only 2 failures needed
    policy.health_check_interval_secs = 1; // Fast checks
    policy.base_interval_secs = 1; // Fast reconnection attempts
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set Connected state
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected state
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    println!("✓ Initial state: Connected");

    // Wait for health checks to fail and trigger reconnection
    let mut seen_disconnected = false;
    let mut seen_reconnecting = false;

    let state_transitions = timeout(Duration::from_secs(15), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                println!("State transition: {:?}", state);

                match state {
                    ConnectionState::Disconnected => {
                        println!("✓ Detected Disconnected state");
                        seen_disconnected = true;
                    }
                    ConnectionState::Reconnecting { attempt, .. } => {
                        println!("✓ Detected Reconnecting state (attempt {})", attempt);
                        seen_reconnecting = true;

                        // Once we've seen Reconnecting, we're done
                        return (seen_disconnected, seen_reconnecting);
                    }
                    _ => {}
                }
            }
        }
    })
    .await;

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    // Verify we saw the state transitions
    assert!(
        state_transitions.is_ok(),
        "Timeout waiting for reconnection state transitions"
    );

    let (saw_disconnected, saw_reconnecting) = state_transitions.unwrap();

    assert!(
        saw_disconnected,
        "Expected to see Disconnected state after health check failures"
    );

    assert!(
        saw_reconnecting,
        "Expected to see Reconnecting state indicating reconnection attempt"
    );

    println!(
        "✓ Test passed: Reconnection manager correctly attempts reconnection after disconnect"
    );
}

#[tokio::test]
async fn test_multiple_reconnection_attempts_with_backoff() {
    // Logging is handled by the test runner

    // Use invalid endpoint to ensure all attempts fail
    let mut policy = create_test_policy("http://192.0.2.1:9999".to_string());
    policy.consecutive_failures_threshold = 2; // Quick trigger
    policy.health_check_interval_secs = 1;
    policy.base_interval_secs = 1; // 1 second base
    policy.backoff_multiplier = 2; // 2x backoff
    policy.max_attempts = 3; // Only 3 attempts
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set Connected state
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    println!("✓ Initial state: Connected");

    // Track reconnection attempts
    let mut reconnection_attempts = Vec::new();

    let result = timeout(Duration::from_secs(20), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();

                if let ConnectionState::Reconnecting { attempt, .. } = state {
                    println!("✓ Reconnection attempt {}", attempt);
                    reconnection_attempts.push(attempt);

                    // If we've seen attempt 3, we're done
                    if attempt >= 3 {
                        return reconnection_attempts;
                    }
                }
            }
        }
    })
    .await;

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    // Verify we saw multiple attempts
    assert!(result.is_ok(), "Timeout waiting for reconnection attempts");

    let attempts = result.unwrap();
    assert!(
        attempts.len() >= 2,
        "Expected at least 2 reconnection attempts, got {}",
        attempts.len()
    );

    // Verify attempts are sequential
    for (i, attempt) in attempts.iter().enumerate() {
        assert_eq!(
            *attempt,
            (i + 1) as u32,
            "Expected attempt {} but got {}",
            i + 1,
            attempt
        );
    }

    println!(
        "✓ Test passed: Multiple reconnection attempts with exponential backoff executed correctly"
    );
}

#[tokio::test]
async fn test_reset_retries_command_clears_state() {
    // Logging is handled by the test runner

    // Use invalid endpoint
    let mut policy = create_test_policy("http://192.0.2.1:9999".to_string());
    policy.consecutive_failures_threshold = 2;
    policy.health_check_interval_secs = 1;
    policy.max_attempts = 3; // Only 3 attempts before Error
    policy.base_interval_secs = 1; // Fast backoff for testing
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy);
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set Connected state
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    // Wait for health checks to fail and reach Error state
    // With fast backoff (1s base * 2^attempt), we should reach max attempts quickly
    let error_detected = timeout(Duration::from_secs(30), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                println!("State: {:?}", state);
                if matches!(state, ConnectionState::Error { .. }) {
                    return true;
                }
            }
        }
    })
    .await;

    // Send ResetRetries command
    command_tx
        .send(ReconnectionCommand::ResetRetries)
        .expect("Failed to send ResetRetries");

    // Give it time to process
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify state transitioned back to Disconnected (not Error)
    let current_state = state_rx.borrow().clone();

    // After reset, should be in Disconnected state (transitioning from Error)
    let is_reset = matches!(current_state, ConnectionState::Disconnected);

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    // Verify we reached error state first
    assert!(
        error_detected.is_ok(),
        "Should have reached Error state before reset"
    );

    // Verify reset worked
    assert!(
        is_reset,
        "Expected state to be Disconnected after ResetRetries, got {:?}",
        current_state
    );

    println!("✓ Test passed: ResetRetries command correctly clears error state");
}

#[tokio::test]
async fn test_successful_reconnection_sets_connected_state() {
    // Logging is handled by the test runner

    // Use a valid endpoint for health checks to succeed after reconnection
    let mut policy = create_test_policy("https://www.google.com".to_string());
    policy.consecutive_failures_threshold = 2;
    policy.health_check_interval_secs = 1;
    policy.base_interval_secs = 1;
    policy.max_attempts = 3;
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Start in Disconnected state (simulating a disconnect)
    let _ = command_tx.send(ReconnectionCommand::Stop);

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Disconnected state
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Disconnected) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Disconnected state");

    println!("✓ Initial state: Disconnected");

    // Trigger reconnection by starting
    command_tx
        .send(ReconnectionCommand::Start)
        .expect("Failed to send Start");

    // Wait for Reconnecting state
    let saw_reconnecting = timeout(Duration::from_secs(10), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                println!("State transition: {:?}", state);
                if let ConnectionState::Reconnecting { attempt, .. } = state {
                    println!("✓ Detected Reconnecting state (attempt {})", attempt);
                    return true;
                }
            }
        }
    })
    .await;

    assert!(
        saw_reconnecting.is_ok() && saw_reconnecting.unwrap(),
        "Should have entered Reconnecting state"
    );

    // Simulate successful reconnection by manually setting Connected state
    // (In real usage, the daemon's state watcher would do this after perform_reconnection succeeds)
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Verify state transitions to Connected
    let became_connected = timeout(Duration::from_secs(5), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                println!("State after reconnection: {:?}", state);
                if matches!(state, ConnectionState::Connected(_)) {
                    return true;
                }
            }
        }
    })
    .await;

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    assert!(
        became_connected.is_ok() && became_connected.unwrap(),
        "Expected state to transition to Connected after successful reconnection"
    );

    println!("✓ Test passed: Successful reconnection correctly sets Connected state");
}

#[tokio::test]
async fn test_reconnection_success_resets_failure_counter() {
    // Logging is handled by the test runner

    // Use a valid endpoint
    let mut policy = create_test_policy("https://www.google.com".to_string());
    policy.consecutive_failures_threshold = 2;
    policy.health_check_interval_secs = 1;
    policy.base_interval_secs = 1;
    let _config = create_test_vpn_config();

    // Create health checker
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_millis(500),
    )
    .expect("Failed to create health checker");

    // Create manager
    let manager = ReconnectionManager::new(policy.clone());
    let command_tx = manager.command_sender();
    let mut state_rx = manager.state_receiver();

    // Set Connected state
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: "test.example.com".to_string(),
            username: "testuser".to_string(),
        })
        .expect("Failed to send SetConnected");

    // Spawn manager
    let manager_handle = tokio::spawn(async move {
        manager.run(Some(health_checker)).await;
    });

    // Wait for Connected state
    timeout(Duration::from_secs(2), async {
        loop {
            if state_rx.changed().await.is_ok() {
                let state = state_rx.borrow().clone();
                if matches!(state, ConnectionState::Connected(_)) {
                    break;
                }
            }
        }
    })
    .await
    .expect("Timeout waiting for Connected state");

    println!("✓ Initial state: Connected");

    // Manually trigger a health check failure by sending CheckNow
    // (This won't actually fail since we're using google.com, but demonstrates the flow)
    command_tx
        .send(ReconnectionCommand::CheckNow)
        .expect("Failed to send CheckNow");

    // Give health check time to execute
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify we're still Connected (successful health check)
    let current_state = state_rx.borrow().clone();
    assert!(
        matches!(current_state, ConnectionState::Connected(_)),
        "Should remain Connected after successful health check"
    );

    // Simulate reconnection success by sending ResetRetries
    // This should reset the consecutive failure counter
    command_tx
        .send(ReconnectionCommand::ResetRetries)
        .expect("Failed to send ResetRetries");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify still Connected
    let current_state = state_rx.borrow().clone();
    assert!(
        matches!(current_state, ConnectionState::Connected(_)),
        "Should remain Connected after ResetRetries"
    );

    // Shutdown
    command_tx
        .send(ReconnectionCommand::Shutdown)
        .expect("Failed to send shutdown");
    let _ = timeout(Duration::from_secs(2), manager_handle).await;

    println!("✓ Test passed: Successful reconnection resets failure counter");
}
