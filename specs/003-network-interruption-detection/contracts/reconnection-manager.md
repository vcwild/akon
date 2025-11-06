# ReconnectionManager Interface Contract

**Module**: `akon-core/src/vpn/reconnection.rs`
**Purpose**: Orchestrate automatic VPN reconnection with exponential backoff

## Interface

```rust
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

/// Manages VPN reconnection lifecycle with exponential backoff
pub struct ReconnectionManager {
    policy: ReconnectionPolicy,
    network_monitor: NetworkMonitor,
    health_checker: HealthChecker,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    command_rx: mpsc::UnboundedReceiver<ReconnectionCommand>,
    command_tx: mpsc::UnboundedSender<ReconnectionCommand>,
}

impl ReconnectionManager {
    /// Create new reconnection manager
    ///
    /// # Arguments
    /// * `policy` - Retry and health check configuration
    /// * `network_monitor` - Network event source
    /// * `health_checker` - VPN health verification
    ///
    /// # Returns
    /// Manager instance with command/state channels
    pub fn new(
        policy: ReconnectionPolicy,
        network_monitor: NetworkMonitor,
        health_checker: HealthChecker,
    ) -> Self;

    /// Start reconnection management loop
    ///
    /// Runs event loop that:
    /// - Listens for network events from NetworkMonitor
    /// - Schedules periodic health checks
    /// - Handles reconnection attempts with exponential backoff
    /// - Processes manual commands (start/stop/reset)
    /// - Updates connection state
    ///
    /// # Returns
    /// Never returns unless shutdown command received
    pub async fn run(mut self) -> Result<(), ReconnectionError>;

    /// Get state subscription for observing connection state changes
    ///
    /// # Returns
    /// Watch receiver that broadcasts state updates
    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState>;

    /// Get command sender for controlling reconnection behavior
    ///
    /// # Returns
    /// Channel sender for ReconnectionCommand messages
    pub fn command_sender(&self) -> mpsc::UnboundedSender<ReconnectionCommand>;

    /// Attempt single VPN reconnection
    ///
    /// Used by run() loop to execute connection attempt.
    /// Checks network stability before connecting.
    ///
    /// # Returns
    /// Ok(true) if connection successful, Ok(false) if network unstable, Err on failure
    async fn attempt_reconnect(&mut self) -> Result<bool, ReconnectionError>;

    /// Calculate next retry delay using exponential backoff
    ///
    /// # Arguments
    /// * `attempt` - Current attempt number (1-indexed)
    ///
    /// # Returns
    /// Duration to wait before next attempt
    fn calculate_backoff(&self, attempt: u32) -> Duration;

    /// Handle network event and update state
    async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<(), ReconnectionError>;

    /// Perform health check and update state
    async fn handle_health_check(&mut self) -> Result<(), ReconnectionError>;
}

/// Commands to control reconnection manager
#[derive(Debug, Clone)]
pub enum ReconnectionCommand {
    /// Start automatic reconnection
    Start,

    /// Stop reconnection attempts
    Stop,

    /// Reset retry counter
    ResetRetries,

    /// Trigger immediate health check
    CheckNow,

    /// Shutdown manager
    Shutdown,
}

#[derive(Debug, thiserror::Error)]
pub enum ReconnectionError {
    #[error("VPN connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Network monitor error: {0}")]
    NetworkMonitorError(#[from] NetworkMonitorError),

    #[error("State persistence error: {0}")]
    StatePersistenceError(#[from] std::io::Error),

    #[error("Max reconnection attempts exceeded")]
    MaxAttemptsExceeded,

    #[error("Reconnection aborted by user")]
    Aborted,
}
```

## State Machine

The manager implements the state machine defined in data-model.md with these transitions:

```
Disconnected → Reconnecting (on network event or failed health check)
Reconnecting → Connected (on successful reconnection)
Reconnecting → Disconnected (on max attempts exceeded)
Reconnecting → Reconnecting (on failed attempt, increment counter)
Connected → Reconnecting (on failed health check)
Connected → Disconnected (on user disconnect command)
```

## Behavior Specification

### Event Loop

The `run()` method implements an async event loop using `tokio::select!`:

```rust
loop {
    tokio::select! {
        // Network event from NetworkMonitor
        Some(event) = network_events.recv() => {
            self.handle_network_event(event).await?;
        }

        // Periodic health check timer
        _ = health_check_interval.tick() => {
            self.handle_health_check().await?;
        }

        // Retry timer for next reconnection attempt
        _ = retry_timer.tick(), if self.state.is_reconnecting() => {
            if let Err(e) = self.attempt_reconnect().await {
                self.handle_reconnection_failure(e).await?;
            }
        }

        // Manual commands
        Some(cmd) = self.command_rx.recv() => {
            self.handle_command(cmd).await?;
        }

        else => break,
    }
}
```

### Reconnection Logic

```rust
async fn attempt_reconnect(&mut self) -> Result<bool, ReconnectionError> {
    // 1. Check network stability
    if !self.health_checker.is_reachable().await {
        tracing::info!("Network not stable, delaying reconnection");
        return Ok(false);
    }

    // 2. Get current attempt number from state
    let attempt = match self.state_rx.borrow().clone() {
        ConnectionState::Reconnecting { attempt, .. } => attempt,
        _ => 1,
    };

    // 3. Update state: next_retry_at = now + backoff
    let next_retry = Instant::now() + self.calculate_backoff(attempt);
    self.state_tx.send(ConnectionState::Reconnecting {
        attempt,
        next_retry_at: Some(next_retry.elapsed().as_secs()),
        max_attempts: self.policy.max_attempts,
    })?;

    // 4. Execute VPN connection via existing VPN module
    match vpn::connect().await {
        Ok(_) => {
            tracing::info!("Reconnection successful on attempt {}", attempt);
            self.state_tx.send(ConnectionState::Connected)?;
            Ok(true)
        }
        Err(e) => {
            tracing::warn!("Reconnection failed on attempt {}: {}", attempt, e);

            if attempt >= self.policy.max_attempts {
                self.state_tx.send(ConnectionState::Disconnected)?;
                Err(ReconnectionError::MaxAttemptsExceeded)
            } else {
                // Increment attempt counter
                self.state_tx.send(ConnectionState::Reconnecting {
                    attempt: attempt + 1,
                    next_retry_at: Some(next_retry.elapsed().as_secs()),
                    max_attempts: self.policy.max_attempts,
                })?;
                Ok(false)
            }
        }
    }
}
```

### Exponential Backoff

```rust
fn calculate_backoff(&self, attempt: u32) -> Duration {
    let base = self.policy.base_interval_secs;
    let multiplier = self.policy.backoff_multiplier;
    let max = self.policy.max_interval_secs;

    let interval_secs = (base as f64 * multiplier.powi((attempt - 1) as i32)) as u64;
    let capped_secs = interval_secs.min(max);

    Duration::from_secs(capped_secs)
}
```

**Example**: With base=5, multiplier=2, max=60:
- Attempt 1: 5s
- Attempt 2: 10s
- Attempt 3: 20s
- Attempt 4: 40s
- Attempt 5+: 60s (capped)

### Health Check Handling

```rust
async fn handle_health_check(&mut self) -> Result<(), ReconnectionError> {
    let result = self.health_checker.check().await;

    let current_state = self.state_rx.borrow().clone();

    match (result.is_healthy(), current_state) {
        // Connected + healthy → no action
        (true, ConnectionState::Connected) => {
            tracing::trace!("Health check passed, connection stable");
        }

        // Connected + unhealthy → trigger reconnection
        (false, ConnectionState::Connected) => {
            tracing::warn!("Health check failed, initiating reconnection");
            self.state_tx.send(ConnectionState::Reconnecting {
                attempt: 1,
                next_retry_at: None,
                max_attempts: self.policy.max_attempts,
            })?;
        }

        // Reconnecting + healthy → might be false positive, continue attempts
        (true, ConnectionState::Reconnecting { .. }) => {
            tracing::debug!("Health check passed during reconnection");
        }

        // Other combinations → no state change
        _ => {}
    }

    Ok(())
}
```

## Testing Contract

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exponential_backoff_calculation() {
        let policy = ReconnectionPolicy {
            max_attempts: 5,
            base_interval_secs: 5,
            backoff_multiplier: 2.0,
            max_interval_secs: 60,
            ..Default::default()
        };

        let manager = create_test_manager(policy);

        assert_eq!(manager.calculate_backoff(1), Duration::from_secs(5));
        assert_eq!(manager.calculate_backoff(2), Duration::from_secs(10));
        assert_eq!(manager.calculate_backoff(3), Duration::from_secs(20));
        assert_eq!(manager.calculate_backoff(4), Duration::from_secs(40));
        assert_eq!(manager.calculate_backoff(5), Duration::from_secs(60)); // Capped
        assert_eq!(manager.calculate_backoff(6), Duration::from_secs(60)); // Still capped
    }

    #[tokio::test]
    async fn test_successful_reconnection_updates_state() {
        let (manager, mut state_rx) = create_test_manager_with_mocks();

        // Given: Manager in Reconnecting state
        manager.state_tx.send(ConnectionState::Reconnecting {
            attempt: 1,
            next_retry_at: None,
            max_attempts: 5,
        }).unwrap();

        // When: Reconnection succeeds
        let result = manager.attempt_reconnect().await;

        // Then: State transitions to Connected
        assert!(result.is_ok());
        assert!(matches!(
            *state_rx.borrow(),
            ConnectionState::Connected
        ));
    }

    #[tokio::test]
    async fn test_max_attempts_exceeded() {
        let policy = ReconnectionPolicy {
            max_attempts: 3,
            ..Default::default()
        };
        let (mut manager, mut state_rx) = create_test_manager_with_mocks(policy);

        // Given: Manager at max attempts
        manager.state_tx.send(ConnectionState::Reconnecting {
            attempt: 3,
            next_retry_at: None,
            max_attempts: 3,
        }).unwrap();

        // When: Reconnection fails
        let result = manager.attempt_reconnect().await;

        // Then: Error returned and state is Disconnected
        assert!(matches!(result, Err(ReconnectionError::MaxAttemptsExceeded)));
        assert!(matches!(
            *state_rx.borrow(),
            ConnectionState::Disconnected
        ));
    }

    #[tokio::test]
    async fn test_network_down_triggers_reconnection() {
        let (mut manager, mut state_rx) = create_test_manager_with_mocks();

        // Given: Connected state
        manager.state_tx.send(ConnectionState::Connected).unwrap();

        // When: NetworkDown event received
        manager.handle_network_event(NetworkEvent::NetworkDown).await.unwrap();

        // Then: State transitions to Reconnecting
        assert!(matches!(
            *state_rx.borrow(),
            ConnectionState::Reconnecting { attempt: 1, .. }
        ));
    }

    #[tokio::test]
    async fn test_health_check_failure_triggers_reconnection() {
        let (mut manager, mut state_rx) = create_test_manager_with_mocks();

        // Given: Connected state and failing health checker
        manager.state_tx.send(ConnectionState::Connected).unwrap();
        // Mock health_checker to return failure

        // When: Health check performed
        manager.handle_health_check().await.unwrap();

        // Then: State transitions to Reconnecting
        assert!(matches!(
            *state_rx.borrow(),
            ConnectionState::Reconnecting { attempt: 1, .. }
        ));
    }
}
```

### Integration Tests

- Test full reconnection lifecycle with mock VPN
- Verify state persistence across restarts
- Test command handling (start/stop/reset)
- Verify backoff timing accuracy

## Dependencies

```toml
[dependencies]
tokio = { version = "1.35", features = ["sync", "time", "macros"] }
thiserror = "1.0"
tracing = "0.1"
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"
```

## Usage Example

```rust
use akon_core::vpn::{ReconnectionManager, ReconnectionPolicy, NetworkMonitor, HealthChecker};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let policy = ReconnectionPolicy::load_from_config()?;

    // Initialize components
    let network_monitor = NetworkMonitor::new().await?;
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Some(Duration::from_secs(policy.health_check_timeout_secs))
    )?;

    // Create manager
    let manager = ReconnectionManager::new(
        policy,
        network_monitor,
        health_checker,
    );

    // Subscribe to state changes
    let mut state_rx = manager.subscribe_state();
    tokio::spawn(async move {
        while state_rx.changed().await.is_ok() {
            let state = state_rx.borrow().clone();
            tracing::info!("VPN state changed: {:?}", state);
        }
    });

    // Get command sender for CLI
    let cmd_tx = manager.command_sender();

    // Run manager (blocks until shutdown)
    manager.run().await?;

    Ok(())
}
```

## Performance Requirements

- **Event latency**: Handle network events within 1 second
- **State update latency**: Broadcast state changes within 100ms
- **CPU overhead**: < 1% CPU when idle (only periodic health checks)
- **Memory overhead**: < 5MB per manager instance
- **Timer accuracy**: Backoff delays accurate within 500ms

## Security Considerations

- **State file permissions**: Ensure state file is user-readable only (0600)
- **No credential storage**: Manager never stores passwords/tokens
- **Graceful shutdown**: Save state on SIGTERM/SIGINT
- **Resource limits**: Limit max health check frequency to prevent DoS

## State Persistence

```rust
// Save state on every transition
async fn persist_state(&self) -> Result<(), std::io::Error> {
    let state_path = get_config_dir()?.join("reconnection_state.toml");
    let state = ReconnectionState {
        current_state: self.state_rx.borrow().clone(),
        last_updated: SystemTime::now(),
    };
    let toml_str = toml::to_string(&state)?;
    tokio::fs::write(state_path, toml_str).await?;
    Ok(())
}

// Restore state on startup
async fn restore_state(&mut self) -> Result<(), std::io::Error> {
    let state_path = get_config_dir()?.join("reconnection_state.toml");
    if state_path.exists() {
        let toml_str = tokio::fs::read_to_string(state_path).await?;
        let state: ReconnectionState = toml::from_str(&toml_str)?;
        self.state_tx.send(state.current_state)?;
    }
    Ok(())
}
```

## Logging

```rust
// State transitions
tracing::info!(
    from = ?old_state,
    to = ?new_state,
    "VPN state transition"
);

// Reconnection attempts
tracing::info!(
    attempt = attempt,
    max_attempts = max_attempts,
    next_retry_secs = next_retry.as_secs(),
    "Scheduling reconnection attempt"
);

// Backoff calculation
tracing::debug!(
    attempt = attempt,
    backoff_secs = backoff.as_secs(),
    "Calculated exponential backoff"
);

// Max attempts exceeded
tracing::error!(
    attempts = max_attempts,
    "Max reconnection attempts exceeded, giving up"
);
```
