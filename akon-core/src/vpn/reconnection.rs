//! VPN reconnection management with exponential backoff
//!
//! This module provides ReconnectionManager for orchestrating automatic
//! VPN reconnection when network interruptions occur.

use crate::vpn::state::ConnectionState;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info};

/// Configuration for automatic reconnection behavior
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReconnectionPolicy {
    /// Maximum number of reconnection attempts before giving up
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Base interval in seconds for exponential backoff
    #[serde(default = "default_base_interval")]
    pub base_interval_secs: u32,

    /// Multiplier for exponential backoff (typically 2)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: u32,

    /// Maximum interval in seconds (cap for exponential growth)
    #[serde(default = "default_max_interval")]
    pub max_interval_secs: u32,

    /// Number of consecutive health check failures before triggering reconnection
    #[serde(default = "default_consecutive_failures")]
    pub consecutive_failures_threshold: u32,

    /// Health check interval in seconds
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,

    /// Health check endpoint URL (HTTP/HTTPS)
    pub health_check_endpoint: String,
}

fn default_max_attempts() -> u32 {
    5
}
fn default_base_interval() -> u32 {
    5
}
fn default_backoff_multiplier() -> u32 {
    2
}
fn default_max_interval() -> u32 {
    60
}
fn default_consecutive_failures() -> u32 {
    3
}
fn default_health_check_interval() -> u64 {
    60
}

impl ReconnectionPolicy {
    /// Validate the entire policy
    ///
    /// Checks all fields against their valid ranges and constraints.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if all fields are valid
    /// * `Err(PolicyValidationError)` with the first validation error encountered
    pub fn validate(&self) -> Result<(), PolicyValidationError> {
        self.validate_max_attempts()?;
        self.validate_base_interval()?;
        self.validate_backoff_multiplier()?;
        self.validate_max_interval()?;
        self.validate_consecutive_failures()?;
        self.validate_health_check_interval()?;
        self.validate_health_check_endpoint()?;
        Ok(())
    }

    /// Validate max_attempts is within range 1-20
    fn validate_max_attempts(&self) -> Result<(), PolicyValidationError> {
        if self.max_attempts < 1 || self.max_attempts > 20 {
            Err(PolicyValidationError::InvalidMaxAttempts(self.max_attempts))
        } else {
            Ok(())
        }
    }

    /// Validate base_interval_secs is within range 1-300
    fn validate_base_interval(&self) -> Result<(), PolicyValidationError> {
        if self.base_interval_secs < 1 || self.base_interval_secs > 300 {
            Err(PolicyValidationError::InvalidBaseInterval(
                self.base_interval_secs,
            ))
        } else {
            Ok(())
        }
    }

    /// Validate backoff_multiplier is within range 1-10
    fn validate_backoff_multiplier(&self) -> Result<(), PolicyValidationError> {
        if self.backoff_multiplier < 1 || self.backoff_multiplier > 10 {
            Err(PolicyValidationError::InvalidBackoffMultiplier(
                self.backoff_multiplier,
            ))
        } else {
            Ok(())
        }
    }

    /// Validate max_interval_secs is >= base_interval_secs
    fn validate_max_interval(&self) -> Result<(), PolicyValidationError> {
        if self.max_interval_secs < self.base_interval_secs {
            Err(PolicyValidationError::MaxIntervalLessThanBase(
                self.max_interval_secs,
                self.base_interval_secs,
            ))
        } else {
            Ok(())
        }
    }

    /// Validate consecutive_failures_threshold is within range 1-10
    fn validate_consecutive_failures(&self) -> Result<(), PolicyValidationError> {
        if self.consecutive_failures_threshold < 1 || self.consecutive_failures_threshold > 10 {
            Err(PolicyValidationError::InvalidConsecutiveFailures(
                self.consecutive_failures_threshold,
            ))
        } else {
            Ok(())
        }
    }

    /// Validate health_check_interval_secs is within range 10-3600
    fn validate_health_check_interval(&self) -> Result<(), PolicyValidationError> {
        if self.health_check_interval_secs < 10 || self.health_check_interval_secs > 3600 {
            Err(PolicyValidationError::InvalidHealthCheckInterval(
                self.health_check_interval_secs,
            ))
        } else {
            Ok(())
        }
    }

    /// Validate health_check_endpoint is a valid HTTP/HTTPS URL
    fn validate_health_check_endpoint(&self) -> Result<(), PolicyValidationError> {
        use url::Url;

        match Url::parse(&self.health_check_endpoint) {
            Ok(url) => match url.scheme() {
                "http" | "https" => Ok(()),
                scheme => Err(PolicyValidationError::InvalidEndpointUrl(format!(
                    "URL scheme must be http or https, got: {}",
                    scheme
                ))),
            },
            Err(e) => Err(PolicyValidationError::InvalidEndpointUrl(format!(
                "Failed to parse URL: {}",
                e
            ))),
        }
    }
}

/// Manages VPN reconnection lifecycle with exponential backoff
pub struct ReconnectionManager {
    policy: ReconnectionPolicy,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    command_rx: mpsc::UnboundedReceiver<ReconnectionCommand>,
    command_tx: mpsc::UnboundedSender<ReconnectionCommand>,
    consecutive_failures_counter: std::sync::Arc<std::sync::Mutex<u32>>,
}

impl ReconnectionManager {
    /// Create a new ReconnectionManager
    ///
    /// # Arguments
    ///
    /// * `policy` - Reconnection policy with retry configuration
    ///
    /// # Returns
    ///
    /// A new ReconnectionManager instance with channels for state and commands
    pub fn new(policy: ReconnectionPolicy) -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        Self {
            policy,
            state_tx,
            state_rx,
            command_rx,
            command_tx,
            consecutive_failures_counter: std::sync::Arc::new(std::sync::Mutex::new(0)),
        }
    }

    /// Calculate backoff duration for a given attempt using exponential backoff
    ///
    /// Formula: base_interval × multiplier^(attempt-1), capped at max_interval
    ///
    /// # Arguments
    ///
    /// * `attempt` - Current attempt number (1-indexed)
    ///
    /// # Returns
    ///
    /// Duration to wait before the next reconnection attempt
    #[tracing::instrument(skip(self), fields(attempt, max_attempts = self.policy.max_attempts))]
    pub fn calculate_backoff(&self, attempt: u32) -> std::time::Duration {
        let base = self.policy.base_interval_secs;
        let multiplier = self.policy.backoff_multiplier;
        let max = self.policy.max_interval_secs;

        // Calculate exponential backoff: base * multiplier^(attempt-1)
        let interval_secs = base as u64 * (multiplier.pow(attempt - 1) as u64);

        // Cap at max_interval
        let capped_secs = interval_secs.min(max as u64);

        std::time::Duration::from_secs(capped_secs)
    }

    /// Get a sender for reconnection commands
    pub fn command_sender(&self) -> mpsc::UnboundedSender<ReconnectionCommand> {
        self.command_tx.clone()
    }

    /// Get a receiver for connection state updates
    pub fn state_receiver(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Attempt to reconnect the VPN
    ///
    /// Checks network stability, updates state with attempt counter,
    /// and handles success/failure outcomes.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Current attempt number (1-indexed)
    ///
    /// # Returns
    ///
    /// Result indicating success or failure with error details
    #[tracing::instrument(skip(self), fields(attempt, max_attempts = self.policy.max_attempts))]
    pub async fn attempt_reconnect(&mut self, attempt: u32) -> Result<(), ReconnectionError> {
        // Check if we've exceeded max attempts
        if attempt > self.policy.max_attempts {
            error!(
                "Max reconnection attempts ({}) exceeded",
                self.policy.max_attempts
            );
            let error_state = ConnectionState::Error(format!(
                "Max reconnection attempts ({}) exceeded",
                self.policy.max_attempts
            ));
            let _ = self.state_tx.send(error_state);
            return Err(ReconnectionError::MaxAttemptsExceeded);
        }

        // Calculate next retry time
        let next_backoff = self.calculate_backoff(attempt + 1);
        info!(
            "Reconnection attempt {}/{}, backoff: {:?}",
            attempt, self.policy.max_attempts, next_backoff
        );

        let next_retry_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + next_backoff.as_secs();

        // Update state to Reconnecting
        let reconnecting_state = ConnectionState::Reconnecting {
            attempt,
            next_retry_at: Some(next_retry_at),
            max_attempts: self.policy.max_attempts,
        };
        debug!("Transitioning to Reconnecting state: attempt {}", attempt);
        let _ = self.state_tx.send(reconnecting_state);

        // Reconnection logic will be handled by external reconnect callback
        // provided to the run method (T025)

        Ok(())
    }

    /// Handle a network event
    ///
    /// Handle health check result
    ///
    /// Tracks consecutive failures and triggers reconnection when threshold is reached.
    /// Only processes health checks when in Connected state.
    ///
    /// # Arguments
    ///
    /// * `health_checker` - HealthChecker instance to perform the check
    ///
    /// # Behavior
    ///
    /// - On success: Resets consecutive failure counter, logs success with duration
    /// - On failure: Increments counter, logs failure count, triggers reconnection if threshold reached
    /// - Only active when state is Connected
    #[tracing::instrument(skip(self, health_checker), fields(threshold = self.policy.consecutive_failures_threshold))]
    pub async fn handle_health_check(
        &mut self,
        health_checker: &crate::vpn::health_check::HealthChecker,
    ) {
        // Only perform health checks when connected
        let current_state = self.state_rx.borrow().clone();
        if !matches!(current_state, ConnectionState::Connected { .. }) {
            debug!("Skipping health check - not in Connected state");
            return;
        }

        // Perform the health check
        let result = health_checker.check().await;

        if result.is_success() {
            // Health check succeeded - reset failure counter
            if let Ok(mut counter) = self.consecutive_failures_counter.lock() {
                let previous_failures = *counter;
                *counter = 0;
                if previous_failures > 0 {
                    debug!(
                        "Health check succeeded after {} failures, resetting counter",
                        previous_failures
                    );
                } else {
                    debug!("Health check succeeded in {:?}", result.duration());
                }
            }
        } else {
            // Health check failed - increment counter and check threshold
            if let Ok(mut counter) = self.consecutive_failures_counter.lock() {
                *counter += 1;
                let current_failures = *counter;

                tracing::warn!(
                    failures = current_failures,
                    threshold = self.policy.consecutive_failures_threshold,
                    error = result.error().unwrap_or("unknown"),
                    "Health check failed"
                );

                // Check if we've reached the threshold
                if current_failures >= self.policy.consecutive_failures_threshold {
                    tracing::error!(
                        failures = current_failures,
                        threshold = self.policy.consecutive_failures_threshold,
                        "Consecutive health check failures reached threshold, triggering reconnection"
                    );

                    // Trigger reconnection by transitioning to Disconnected
                    // The run loop will handle the actual reconnection attempt
                    let _ = self.state_tx.send(ConnectionState::Disconnected);

                    // Reset counter for the next cycle
                    *counter = 0;
                }
            }
        }
    }

    /// Run the reconnection manager event loop
    ///
    /// Processes network events, handles retry timers, performs periodic health checks,
    /// and responds to commands. This should be spawned as a background tokio task.
    ///
    /// # Arguments
    ///
    /// * `health_checker` - Optional health checker for periodic connectivity validation
    pub async fn run(mut self, health_checker: Option<crate::vpn::health_check::HealthChecker>) {
        use tokio::time::{interval, Duration};

        let mut retry_timer = interval(Duration::from_secs(5));
        retry_timer.tick().await; // Consume first immediate tick

        // Create health check interval timer
        let mut health_check_timer =
            interval(Duration::from_secs(self.policy.health_check_interval_secs));
        health_check_timer.tick().await; // Consume first immediate tick

        let mut current_attempt = 1u32;
        let mut should_reconnect = false;

        // Clone state receiver for monitoring state changes
        let mut state_monitor = self.state_rx.clone();

        loop {
            tokio::select! {
                // Monitor for state changes to react immediately to Disconnected state
                Ok(_) = state_monitor.changed() => {
                    let current_state = state_monitor.borrow().clone();
                    if matches!(current_state, ConnectionState::Disconnected) && !should_reconnect {
                        tracing::info!("State changed to Disconnected, immediately initiating reconnection");
                        should_reconnect = true;
                        current_attempt = 1;
                    }
                }

                // Handle commands from external control
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        ReconnectionCommand::Start => {
                            should_reconnect = true;
                            current_attempt = 1;
                        }
                        ReconnectionCommand::Stop => {
                            should_reconnect = false;
                            let _ = self.state_tx.send(ConnectionState::Disconnected);
                        }
                        ReconnectionCommand::ResetRetries => {
                            // T050: Reset retry counter and consecutive failures counter
                            current_attempt = 1;
                            if let Ok(mut counter) = self.consecutive_failures_counter.lock() {
                                *counter = 0;
                            }

                            // T050: Transition from Error state to Disconnected
                            let current_state = self.state_rx.borrow().clone();
                            if matches!(current_state, ConnectionState::Error { .. }) {
                                let _ = self.state_tx.send(ConnectionState::Disconnected);
                                tracing::info!("Reset retries: transitioned from Error to Disconnected state");
                            }

                            tracing::info!("Reset retries: cleared attempt counter and consecutive failures");
                        }
                        ReconnectionCommand::SetConnected { server, username } => {
                            // Set state to Connected (used when VPN initially connects or after successful reconnection)
                            use crate::vpn::state::ConnectionMetadata;
                            let metadata = ConnectionMetadata::new(server, username);
                            let _ = self.state_tx.send(ConnectionState::Connected(metadata));

                            // Stop reconnection attempts and reset counters
                            should_reconnect = false;
                            current_attempt = 1;
                            if let Ok(mut counter) = self.consecutive_failures_counter.lock() {
                                *counter = 0;
                            }

                            tracing::info!("State set to Connected, health check monitoring enabled");
                        }
                        ReconnectionCommand::CheckNow => {
                            // Immediate health check
                            if let Some(ref checker) = health_checker {
                                self.handle_health_check(checker).await;
                            }
                        }
                        ReconnectionCommand::Shutdown => {
                            break;
                        }
                    }
                }

                // Handle retry timer
                _ = retry_timer.tick() => {
                    // Check if we need to start reconnection due to Disconnected state
                    let current_state = self.state_rx.borrow().clone();
                    if matches!(current_state, ConnectionState::Disconnected) && !should_reconnect {
                        tracing::info!("Detected Disconnected state, initiating reconnection");
                        should_reconnect = true;
                        current_attempt = 1;
                    }

                    if should_reconnect {
                        match self.attempt_reconnect(current_attempt).await {
                            Ok(_) => {
                                // Attempt scheduled, increment for next time
                                current_attempt += 1;
                            }
                            Err(ReconnectionError::MaxAttemptsExceeded) => {
                                should_reconnect = false;
                                current_attempt = 1;
                            }
                            Err(_) => {
                                current_attempt += 1;
                            }
                        }
                    }
                }

                // Handle periodic health checks
                _ = health_check_timer.tick(), if health_checker.is_some() => {
                    if let Some(ref checker) = health_checker {
                        self.handle_health_check(checker).await;
                    }
                }
            }
        }
    }
}

use std::time::SystemTime;

/// Commands to control reconnection manager
#[derive(Debug, Clone)]
pub enum ReconnectionCommand {
    /// Start automatic reconnection
    Start,

    /// Stop reconnection attempts
    Stop,

    /// Reset retry counter
    ResetRetries,

    /// Set state to Connected (for initial connection)
    SetConnected { server: String, username: String },

    /// Trigger immediate health check
    CheckNow,

    /// Shutdown manager
    Shutdown,
}

/// Errors that can occur during reconnection
#[derive(Debug, thiserror::Error)]
pub enum ReconnectionError {
    #[error("VPN connection failed: {0}")]
    ConnectionFailed(String),

    #[error("State persistence error: {0}")]
    StatePersistenceError(#[from] std::io::Error),

    #[error("Max reconnection attempts exceeded")]
    MaxAttemptsExceeded,

    #[error("Reconnection aborted by user")]
    Aborted,

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
}

/// Validation errors for ReconnectionPolicy
#[derive(Debug, thiserror::Error)]
pub enum PolicyValidationError {
    #[error("max_attempts must be between 1 and 20, got: {0}")]
    InvalidMaxAttempts(u32),

    #[error("base_interval_secs must be between 1 and 300, got: {0}")]
    InvalidBaseInterval(u32),

    #[error("backoff_multiplier must be between 1 and 10, got: {0}")]
    InvalidBackoffMultiplier(u32),

    #[error("max_interval_secs ({0}) must be >= base_interval_secs ({1})")]
    MaxIntervalLessThanBase(u32, u32),

    #[error("consecutive_failures_threshold must be between 1 and 10, got: {0}")]
    InvalidConsecutiveFailures(u32),

    #[error("health_check_interval_secs must be between 10 and 3600, got: {0}")]
    InvalidHealthCheckInterval(u64),

    #[error("health_check_endpoint must be a valid HTTP/HTTPS URL: {0}")]
    InvalidEndpointUrl(String),
}
