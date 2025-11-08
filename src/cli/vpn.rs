//! VPN connection management commands
//!
//! CLI-based OpenConnect integration using process delegation

use crate::daemon::process::cleanup_orphaned_processes;
use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::{get_config_path, TomlConfig};
use akon_core::error::{AkonError, VpnError};
use akon_core::vpn::health_check::HealthChecker;
use akon_core::vpn::reconnection::ReconnectionManager;
use akon_core::vpn::{CliConnector, ConnectionEvent};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// State file for tracking VPN connection
fn state_file_path() -> PathBuf {
    std::env::var("AKON_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/akon_vpn_state.json"))
}

/// Handle cleanup_orphaned_processes result with user feedback
fn handle_cleanup_result(result: Result<usize, AkonError>, context: &str) {
    match result {
        Ok(0) => {
            println!("  {} No orphaned processes found", "✓".bright_green());
            debug!("{}: No orphaned OpenConnect processes to clean up", context);
        }
        Ok(count) => {
            println!(
                "  {} Terminated {} orphaned process(es)",
                "✓".bright_green(),
                count.to_string().bright_yellow()
            );
            info!(
                count,
                "{}: Terminated orphaned OpenConnect processes", context
            );
        }
        Err(e) => {
            warn!("{}: Orphan cleanup failed: {}", context, e);
            println!(
                "  {} Warning: Could not verify all processes cleaned up",
                "⚠".bright_yellow()
            );
        }
    }
}

/// Print actionable suggestions based on VPN error type
fn print_error_suggestions(error: &VpnError) {
    match error {
        VpnError::AuthenticationFailed => {
            eprintln!(
                "\n{} {}",
                "💡".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   {} Verify your PIN is correct", "•".bright_blue());
            eprintln!(
                "   {} Check if your TOTP secret is valid",
                "•".bright_blue()
            );
            eprintln!(
                "   {} Run {} to reconfigure credentials",
                "•".bright_blue(),
                "akon setup".bright_cyan()
            );
            eprintln!("   {} Ensure your account is not locked", "•".bright_blue());
        }
        VpnError::NetworkError { reason } if reason.contains("SSL") || reason.contains("TLS") => {
            eprintln!("\n💡 Suggestions:");
            eprintln!("   • Check your internet connection");
            eprintln!("   • Verify the VPN server address is correct");
            eprintln!("   • The server may be experiencing issues");
            eprintln!("   • Try again in a few moments");
        }
        VpnError::NetworkError { reason } if reason.contains("Certificate") => {
            eprintln!("\n💡 Suggestions:");
            eprintln!("   • The server certificate may be self-signed");
            eprintln!("   • Contact your VPN administrator for certificate details");
            eprintln!("   • You may need to add the certificate to your trusted store");
        }
        VpnError::NetworkError { reason } if reason.contains("DNS") => {
            eprintln!("\n💡 Suggestions:");
            eprintln!("   • Check your DNS configuration");
            eprintln!("   • Verify the VPN server hostname in config.toml");
            eprintln!("   • Try using the server's IP address instead");
            eprintln!("   • Check /etc/resolv.conf for DNS settings");
        }
        VpnError::ConnectionFailed { reason }
            if reason.contains("TUN") || reason.contains("sudo") =>
        {
            eprintln!("\n💡 Suggestions:");
            eprintln!("   • VPN requires root privileges to create TUN device");
            eprintln!("   • Run with: sudo akon vpn on");
            eprintln!("   • Ensure the 'tun' kernel module is loaded");
            eprintln!("   • Check: lsmod | grep tun");
        }
        VpnError::ProcessSpawnError { .. } => {
            eprintln!(
                "\n{} {}",
                "💡".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   {} OpenConnect may not be installed", "•".bright_blue());
            eprintln!(
                "   {} Install with: {}",
                "•".bright_blue(),
                "sudo apt install openconnect".bright_cyan()
            );
            eprintln!(
                "   {} Or for RHEL/Fedora: {}",
                "•".bright_blue(),
                "sudo dnf install openconnect".bright_cyan()
            );
            eprintln!(
                "   {} Verify installation: {}",
                "•".bright_blue(),
                "which openconnect".bright_cyan()
            );
        }
        VpnError::ConnectionFailed { reason } if reason.contains("Permission denied") => {
            eprintln!(
                "\n{} {}",
                "💡".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!(
                "   {} This command requires elevated privileges",
                "•".bright_blue()
            );
            eprintln!(
                "   {} Run with: {}",
                "•".bright_blue(),
                "sudo akon vpn on".bright_cyan()
            );
        }
        _ => {
            // Generic suggestions for other errors
            eprintln!(
                "\n{} {}",
                "💡".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!(
                "   {} Check system logs: {}",
                "•".bright_blue(),
                "journalctl -xe".bright_cyan()
            );
            eprintln!(
                "   {} Verify configuration: {}",
                "•".bright_blue(),
                "cat ~/.config/akon/config.toml".bright_cyan()
            );
            eprintln!(
                "   {} Try reconnecting: {}",
                "•".bright_blue(),
                "akon vpn on".bright_cyan()
            );
        }
    }
}

/// Perform VPN reconnection by cleaning up stale processes and establishing new connection
async fn perform_reconnection(config: akon_core::config::VpnConfig) -> Result<(), AkonError> {
    info!("Performing VPN reconnection");

    // Step 1: Cleanup all stale OpenConnect processes
    info!("Cleaning up stale OpenConnect processes");

    match cleanup_orphaned_processes() {
        Ok(count) => {
            if count > 0 {
                info!(
                    "Terminated {} orphaned process(es) before reconnection",
                    count
                );
            } else {
                debug!("No orphaned processes found before reconnection");
            }
        }
        Err(e) => {
            warn!("Cleanup failed before reconnection: {}", e);
            // Continue anyway - reconnection might still work
        }
    }

    // Step 2: Wait a moment for cleanup to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Step 3: Generate new password
    let password = generate_password(&config.username).map_err(|e| {
        error!("Failed to generate password for reconnection: {}", e);
        e
    })?;
    info!("Generated password for reconnection");

    // Step 4: Create new connector and establish connection
    let mut connector = akon_core::vpn::CliConnector::new(config.clone())?;
    info!("Created new CLI connector for reconnection");

    // Step 5: Connect
    connector.connect(password.expose().to_string()).await?;
    info!("Reconnection initiated, waiting for connection events");

    // Step 6: Wait for connection to establish
    let timeout_duration = Duration::from_secs(60);
    match tokio::time::timeout(timeout_duration, async {
        while let Some(event) = connector.next_event().await {
            match event {
                akon_core::vpn::ConnectionEvent::Connected { ip, device } => {
                    info!(ip = %ip, device = %device, "Reconnection successful");

                    // Update state file
                    let pid = connector.get_pid();
                    let state = serde_json::json!({
                        "ip": ip.to_string(),
                        "device": device,
                        "connected_at": chrono::Utc::now().to_rfc3339(),
                        "pid": pid,
                    });

                    if let Ok(state_json) = serde_json::to_string_pretty(&state) {
                        let _ = fs::write(state_file_path(), state_json);
                    }

                    return Ok::<(), AkonError>(());
                }
                akon_core::vpn::ConnectionEvent::Error { kind, .. } => {
                    error!("Reconnection failed: {}", kind);
                    return Err(AkonError::Vpn(kind));
                }
                _ => {
                    // Continue processing events
                }
            }
        }
        Err(AkonError::Vpn(VpnError::ConnectionFailed {
            reason: "Connection closed unexpectedly during reconnection".to_string(),
        }))
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            error!("Reconnection timeout after 60 seconds");
            Err(AkonError::Vpn(VpnError::ConnectionTimeout { seconds: 60 }))
        }
    }
}

/// Spawn the reconnection manager as a daemon process
///
/// This function creates a detached background process that manages automatic reconnection by:
/// 1. Performing periodic health checks
/// 2. Triggering reconnection with exponential backoff when health checks fail
/// 3. Killing stale OpenConnect processes before reconnecting
/// 4. Establishing new VPN connection
///
/// The daemon runs independently and can be stopped by killing the VPN connection.
fn spawn_reconnection_manager_daemon(
    policy: akon_core::vpn::reconnection::ReconnectionPolicy,
    config: akon_core::config::VpnConfig,
    _initial_pid: u32,
) -> Result<(), AkonError> {
    use std::process::Command;

    info!("Spawning reconnection manager daemon");

    // Kill any existing reconnection manager daemons before starting a new one
    info!("Cleaning up any existing reconnection manager daemons");
    let _ = Command::new("pkill")
        .arg("-f")
        .arg("__internal_reconnection_daemon")
        .output();

    // Give processes time to terminate
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Get the current executable path
    let exe_path = std::env::current_exe().map_err(|e| {
        error!("Failed to get current executable path: {}", e);
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to get executable path: {}", e),
        })
    })?;

    // Serialize the policy and config to pass to daemon
    let policy_json = serde_json::to_string(&policy).map_err(|e| {
        error!("Failed to serialize reconnection policy: {}", e);
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to serialize policy: {}", e),
        })
    })?;

    let config_json = serde_json::to_string(&config).map_err(|e| {
        error!("Failed to serialize VPN config: {}", e);
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to serialize config: {}", e),
        })
    })?;

    // Spawn the daemon as a detached child process
    let child = Command::new(&exe_path)
        .arg("__internal_reconnection_daemon")
        .arg(&policy_json)
        .arg(&config_json)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            error!("Failed to spawn reconnection manager daemon: {}", e);
            AkonError::Vpn(VpnError::ProcessSpawnError {
                reason: format!("Failed to spawn daemon: {}", e),
            })
        })?;

    info!(
        "Reconnection manager daemon spawned with PID {}",
        child.id()
    );

    // Save daemon PID to file for tracking
    let daemon_pid_file = get_daemon_pid_file();
    if let Err(e) = std::fs::write(&daemon_pid_file, child.id().to_string()) {
        warn!("Failed to write daemon PID file: {}", e);
    }

    Ok(())
}

/// Internal function to run the reconnection manager daemon
/// This is called by the daemon process itself, not by user commands
#[doc(hidden)]
pub async fn run_reconnection_manager_daemon(
    policy: akon_core::vpn::reconnection::ReconnectionPolicy,
    config: akon_core::config::VpnConfig,
) -> Result<(), AkonError> {
    use tokio::time::Duration;

    info!("Reconnection manager daemon starting");

    // Create HealthChecker for periodic connectivity verification
    let health_checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_secs(5), // 5 second timeout per health check
    )
    .map_err(|e| {
        error!("Failed to create HealthChecker: {}", e);
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to initialize health checker: {}", e),
        })
    })?;
    info!(
        "HealthChecker initialized with endpoint: {}, interval: {}s",
        policy.health_check_endpoint, policy.health_check_interval_secs
    );

    // Create ReconnectionManager
    let reconnection_manager = ReconnectionManager::new(policy.clone());
    let command_tx = reconnection_manager.command_sender();
    let mut state_rx = reconnection_manager.state_receiver();
    info!(
        "ReconnectionManager created with max_attempts={}, base_interval={}s, backoff={}x",
        policy.max_attempts, policy.base_interval_secs, policy.backoff_multiplier
    );

    // Set initial state to Connected since VPN is already up
    use akon_core::vpn::reconnection::ReconnectionCommand;
    command_tx
        .send(ReconnectionCommand::SetConnected {
            server: config.server.clone(),
            username: config.username.clone(),
        })
        .ok();
    info!("Set reconnection manager state to Connected");

    // Spawn a task to watch for reconnection state changes and trigger actual reconnection
    let config_for_watcher = config.clone();
    let policy_for_watcher = policy.clone();

    // Track if reconnection is in progress and last attempt number to prevent duplicate attempts
    let reconnection_state = Arc::new(tokio::sync::Mutex::new((false, 0u32))); // (in_progress, last_attempt)
    let reconnection_state_clone = reconnection_state.clone();

    tokio::spawn(async move {
        use akon_core::vpn::reconnection::ReconnectionCommand;
        use akon_core::vpn::state::ConnectionState;

        loop {
            // Wait for state changes
            if state_rx.changed().await.is_err() {
                break;
            }

            let state = state_rx.borrow().clone();

            // T053: Update state file with current reconnection state
            match &state {
                ConnectionState::Reconnecting {
                    attempt,
                    next_retry_at,
                    max_attempts,
                } => {
                    // Check if we should process this attempt
                    let mut reconnection_info = reconnection_state_clone.lock().await;
                    let (in_progress, last_attempt) = *reconnection_info;

                    // Skip if:
                    // 1. A reconnection is already in progress, OR
                    // 2. We've already processed this attempt number
                    if in_progress {
                        info!(
                            "Reconnection already in progress, skipping attempt {}",
                            attempt
                        );
                        let state_json = serde_json::json!({
                            "state": "Reconnecting",
                            "attempt": attempt,
                            "next_retry_at": next_retry_at,
                            "max_attempts": max_attempts,
                            "updated_at": chrono::Utc::now().to_rfc3339(),
                        });
                        if let Ok(json) = serde_json::to_string_pretty(&state_json) {
                            let _ = fs::write(state_file_path(), json);
                        }
                        continue;
                    }

                    if *attempt <= last_attempt {
                        info!("Skipping already processed attempt {}", attempt);
                        continue;
                    }

                    // Mark reconnection as in progress and update last attempt
                    *reconnection_info = (true, *attempt);
                    drop(reconnection_info); // Release lock before async work

                    info!("Starting reconnection attempt {}", attempt);

                    // Write reconnecting state to file
                    let state_json = serde_json::json!({
                        "state": "Reconnecting",
                        "attempt": attempt,
                        "next_retry_at": next_retry_at,
                        "max_attempts": max_attempts,
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                    if let Ok(json) = serde_json::to_string_pretty(&state_json) {
                        let _ = fs::write(state_file_path(), json);
                    }

                    // Perform the actual reconnection
                    match perform_reconnection(config_for_watcher.clone()).await {
                        Ok(_) => {
                            info!(
                                "Reconnection attempt {} successful, transitioning to Connected",
                                attempt
                            );
                            // Set state to Connected to stop the retry loop
                            let _ = command_tx.send(ReconnectionCommand::SetConnected {
                                server: config_for_watcher.server.clone(),
                                username: config_for_watcher.username.clone(),
                            });

                            // Set last_attempt to MAX to reject ALL queued retry attempts
                            // This prevents any queued Reconnecting(attempt=2, 3, 4, 5) states
                            // from being processed after successful reconnection
                            let mut reconnection_info = reconnection_state_clone.lock().await;
                            reconnection_info.0 = false; // Clear in_progress flag
                            reconnection_info.1 = u32::MAX; // Reject all future attempts until reset
                            info!("Set last_attempt=MAX to reject any queued retry attempts");
                        }
                        Err(e) => {
                            warn!("Reconnection attempt {} failed: {}", attempt, e);
                            // Mark reconnection as complete so next attempt can proceed
                            let mut reconnection_info = reconnection_state_clone.lock().await;
                            reconnection_info.0 = false; // Clear in_progress flag
                                                         // Keep last_attempt so we don't retry the same attempt
                        }
                    }
                }
                ConnectionState::Connected(_) => {
                    // When we reach Connected state from SetConnected command,
                    // reset last_attempt to 0 so new disconnections can be handled
                    let mut reconnection_info = reconnection_state_clone.lock().await;
                    if reconnection_info.1 > 0 {
                        info!("Connected state reached, resetting reconnection tracking for future disconnections");
                        *reconnection_info = (false, 0);
                    }
                }
                ConnectionState::Error(error_msg) => {
                    // T053: Write Error state to file so 'akon vpn status' can detect it
                    warn!("Reconnection manager in Error state: {}", error_msg);
                    let state_json = serde_json::json!({
                        "state": "Error",
                        "error": error_msg,
                        "max_attempts": policy_for_watcher.max_attempts,
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                    if let Ok(json) = serde_json::to_string_pretty(&state_json) {
                        let _ = fs::write(state_file_path(), json);
                    }
                }
                ConnectionState::Disconnected => {
                    info!("Reconnection manager in Disconnected state");
                    let state_json = serde_json::json!({
                        "state": "Disconnected",
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                    if let Ok(json) = serde_json::to_string_pretty(&state_json) {
                        let _ = fs::write(state_file_path(), json);
                    }
                }
                _ => {
                    // Other states (Connected, Connecting, Disconnecting) are handled elsewhere
                }
            }
        }
    });

    // Start the reconnection manager event loop with health checking
    info!("Starting reconnection manager event loop (health check mode)");
    reconnection_manager.run(Some(health_checker)).await;

    Ok(())
}

/// Get the path to the daemon PID file
fn get_daemon_pid_file() -> PathBuf {
    // Use /tmp for the daemon PID file
    PathBuf::from("/tmp/akon-reconnection-daemon.pid")
}

/// Stop the reconnection manager daemon
fn stop_reconnection_manager_daemon() {
    let daemon_pid_file = get_daemon_pid_file();

    if !daemon_pid_file.exists() {
        debug!("No reconnection manager daemon running");
        return;
    }

    // Read daemon PID
    let pid_content = match fs::read_to_string(&daemon_pid_file) {
        Ok(content) => content,
        Err(e) => {
            warn!("Failed to read daemon PID file: {}", e);
            return;
        }
    };

    let daemon_pid: i32 = match pid_content.trim().parse() {
        Ok(pid) => pid,
        Err(e) => {
            warn!("Invalid PID in daemon file: {}", e);
            let _ = fs::remove_file(&daemon_pid_file);
            return;
        }
    };

    info!("Stopping reconnection manager daemon (PID: {})", daemon_pid);

    // Send SIGTERM to daemon
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    match kill(Pid::from_raw(daemon_pid), Signal::SIGTERM) {
        Ok(_) => {
            info!("Sent SIGTERM to reconnection manager daemon");
            // Give it a moment to shut down gracefully
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        Err(e) => {
            warn!("Failed to send SIGTERM to daemon: {}", e);
        }
    }

    // Clean up PID file
    if let Err(e) = fs::remove_file(&daemon_pid_file) {
        warn!("Failed to remove daemon PID file: {}", e);
    }
}

/// Run the VPN on command using CLI process delegation
pub async fn run_vpn_on(force: bool) -> Result<(), AkonError> {
    // Check for existing connection first
    let state_path = state_file_path();
    if state_path.exists() {
        // Try to read existing state
        if let Ok(state_content) = fs::read_to_string(&state_path) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_content) {
                if let Some(pid) = state.get("pid").and_then(|p| p.as_u64()) {
                    // Check if process is still running
                    let process_running = std::process::Command::new("ps")
                        .args(["-p", &pid.to_string()])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);

                    if process_running {
                        if force {
                            // Force reconnection - disconnect first and reset state
                            info!(
                                "Force flag set, disconnecting existing connection (PID: {}) and resetting state",
                                pid
                            );
                            println!(
                                "{} {}",
                                "🔄".bright_yellow(),
                                "Force reconnection requested - disconnecting and resetting..."
                                    .bright_yellow()
                            );

                            // Disconnect the existing connection
                            let _ = std::process::Command::new("sudo")
                                .args(["kill", "-TERM", &pid.to_string()])
                                .status();

                            // Wait a moment for graceful shutdown
                            std::thread::sleep(std::time::Duration::from_secs(1));

                            // Force kill if still running
                            let still_running = std::process::Command::new("ps")
                                .args(["-p", &pid.to_string()])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false);

                            if still_running {
                                let _ = std::process::Command::new("sudo")
                                    .args(["kill", "-KILL", &pid.to_string()])
                                    .status();
                            }

                            // Clean up state file (reset functionality)
                            let _ = fs::remove_file(&state_path);
                            println!("  {} Cleared connection state", "✓".bright_green());
                            info!("Force flag cleared state file (reset)");
                        } else {
                            // Connection is already active - return early
                            println!(
                                "{} {}",
                                "✓".bright_green().bold(),
                                "VPN is already connected".bright_green()
                            );
                            if let Some(ip) = state.get("ip") {
                                println!(
                                    "  {} {}",
                                    "IP address:".bright_white(),
                                    ip.as_str().unwrap_or("unknown").bright_cyan().bold()
                                );
                            }
                            println!(
                                "\n{} {} to see full status",
                                "Run".dimmed(),
                                "akon vpn status".bright_cyan()
                            );
                            return Ok(());
                        }
                    } else {
                        // Stale connection - clean up
                        info!("Found stale connection state (PID: {}), cleaning up", pid);
                        println!(
                            "{} {}",
                            "⚠".bright_yellow(),
                            "Cleaning up stale connection...".dimmed()
                        );
                        let _ = fs::remove_file(&state_path);
                    }
                }
            }
        }
    }

    // Load configuration
    let config_path = get_config_path()?;
    let toml_config = TomlConfig::from_file(&config_path)?;
    let config = toml_config.vpn_config;
    info!("Loaded configuration for server: {}", config.server);

    // Generate complete VPN password (PIN + OTP) from user's keyring
    let password = generate_password(&config.username)?;
    info!("Generated VPN password from keyring credentials");

    // Check if OpenConnect is installed
    if let Err(e) = which::which("openconnect") {
        error!("OpenConnect not found in PATH: {}", e);
        eprintln!("Error: OpenConnect is not installed or not in PATH");
        eprintln!("Install it with: sudo apt install openconnect");
        return Err(AkonError::Vpn(VpnError::ProcessSpawnError {
            reason: "openconnect command not found".to_string(),
        }));
    }

    // Create CLI connector
    let mut connector = CliConnector::new(config.clone())?;
    info!("Created CLI connector");

    // Start connection
    println!(
        "{} {} {}",
        "🔌".bright_cyan(),
        "Connecting to VPN server:".bright_white().bold(),
        config.server.bright_yellow()
    );
    connector.connect(password.expose().to_string()).await?;

    // Monitor events
    // Note: We don't use a timeout wrapper here when reconnection is enabled,
    // as the reconnection manager needs to run indefinitely
    let process_result = async {
        while let Some(event) = connector.next_event().await {
            // Log all events with structured metadata (T047)
            info!("Connection event: {:?}", event);

            match event {
                ConnectionEvent::ProcessStarted { pid } => {
                    debug!("OpenConnect process started with PID: {}", pid);
                    info!(pid = pid, "VPN process spawned");
                }
                ConnectionEvent::Authenticating { message } => {
                    println!("{} {}", "🔐".bright_magenta(), message.bright_white());
                    info!(phase = "authentication", message = %message, "Authentication in progress");
                }
                ConnectionEvent::F5SessionEstablished { .. } => {
                    // Silent - not shown to user during connection
                    info!(phase = "session", "F5 session established");
                }
                ConnectionEvent::TunConfigured { device, ip } => {
                    // Silent - not shown to user during connection
                    info!(device = %device, ip = %ip, "TUN device configured");
                }
                ConnectionEvent::Connected { ip, device } => {
                    println!("{} {}", "✓".bright_green().bold(), "VPN connection established".bright_green().bold());
                    info!(ip = %ip, device = %device, "VPN connection fully established");

                    // Get PID from connector for state persistence
                    let pid = connector.get_pid();

                    // Save state for status command
                    let state = serde_json::json!({
                        "ip": ip.to_string(),
                        "device": device,
                        "connected_at": chrono::Utc::now().to_rfc3339(),
                        "pid": pid,
                    });

                    let state_json = serde_json::to_string_pretty(&state).map_err(|e| {
                        AkonError::Vpn(VpnError::ConnectionFailed {
                            reason: format!("Failed to serialize state: {}", e),
                        })
                    })?;

                    if let Err(e) = fs::write(state_file_path(), state_json) {
                        error!("Failed to write state file: {}", e);
                    }

                    // Start reconnection manager daemon if reconnection policy is configured
                    if let Some(reconnection_policy) = toml_config.reconnection.clone() {
                        // Only start if we have a valid PID
                        if let Some(pid_value) = pid {
                            info!("Starting reconnection manager daemon with policy: max_attempts={}, health_endpoint={}",
                                  reconnection_policy.max_attempts,
                                  reconnection_policy.health_check_endpoint);

                            // Spawn the reconnection manager as a daemon
                            let config_for_reconnection = config.clone();
                            if let Err(e) = spawn_reconnection_manager_daemon(
                                reconnection_policy,
                                config_for_reconnection,
                                pid_value
                            ) {
                                error!("Failed to spawn reconnection manager daemon: {}", e);
                                warn!("Continuing without reconnection manager");
                            } else {
                                println!("{} {}", "🔄".bright_cyan(), "Reconnection manager started in background".dimmed());
                            }
                        } else {
                            warn!("Cannot start reconnection manager: no PID available");
                        }
                    } else {
                        debug!("No reconnection policy configured, skipping reconnection manager");
                    }

                    return Ok::<(), AkonError>(());
                }
                ConnectionEvent::Error { kind, raw_output } => {
                    error!("VPN error: {} - {}", kind, raw_output);
                    eprintln!("{} {}", "❌".bright_red(), format!("Error: {}", kind).bright_red().bold());
                    if !raw_output.is_empty() {
                        eprintln!("   {} {}", "Details:".bright_yellow(), raw_output.dimmed());
                    }

                    // Provide actionable suggestions based on error type
                    print_error_suggestions(&kind);

                    return Err(AkonError::Vpn(kind));
                }
                ConnectionEvent::Disconnected { reason } => {
                    info!("VPN disconnected: {:?}", reason);
                    println!("{} VPN disconnected: {:?}", "⚠".bright_yellow(), reason);
                    return Ok(());
                }
                ConnectionEvent::UnknownOutput { line } => {
                    debug!("Unparsed output: {}", line);
                }
            }
        }

        // If we exit the loop without connecting, that's an error
        Err(AkonError::Vpn(VpnError::ConnectionFailed {
            reason: "Connection closed unexpectedly".to_string(),
        }))
    }.await;

    process_result
}

/// Run the VPN off command
///
/// Disconnects from VPN by terminating the tracked OpenConnect process and
/// cleaning up any orphaned OpenConnect processes from previous sessions.
pub async fn run_vpn_off() -> Result<(), AkonError> {
    use nix::unistd::Pid;

    // Load state file
    let state_path = state_file_path();

    if !state_path.exists() {
        println!("No active VPN connection found");

        // Still check for and clean up any orphaned OpenConnect processes
        println!(
            "{} {}",
            "🧹".bright_yellow(),
            "Checking for orphaned OpenConnect processes...".bright_white()
        );

        info!("No active connection, scanning for orphaned processes");

        let result = cleanup_orphaned_processes();
        handle_cleanup_result(result, "run_vpn_off (no state)");

        return Ok(());
    }

    // Read state to get PID
    let state_content = fs::read_to_string(&state_path).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to read state file: {}", e),
        })
    })?;

    let state: serde_json::Value = serde_json::from_str(&state_content).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to parse state file: {}", e),
        })
    })?;

    // Extract PID
    let pid = state.get("pid").and_then(|p| p.as_u64()).ok_or_else(|| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: "PID not found in state file".to_string(),
        })
    })? as i32;

    let pid = Pid::from_raw(pid);

    // Check if process is still running (Step 2 from vpn-off-command.md)
    // Note: openconnect runs as root, so we check via ps and kill with sudo
    let process_running = std::process::Command::new("ps")
        .args(["-p", &pid.as_raw().to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if process_running {
        // Process exists, try graceful termination
        println!(
            "{} {} (PID: {})...",
            "🔌".bright_cyan(),
            "Disconnecting VPN".bright_white().bold(),
            pid.to_string().bright_yellow()
        );
        info!(pid = pid.as_raw(), "Sending SIGTERM to OpenConnect process");

        // Send SIGTERM via sudo (Step 3)
        let kill_result = std::process::Command::new("sudo")
            .args(["kill", "-TERM", &pid.as_raw().to_string()])
            .status();

        if let Err(e) = kill_result {
            error!("Failed to send SIGTERM: {}", e);
            return Err(AkonError::Vpn(VpnError::TerminationError));
        }

        // Wait up to 5 seconds for graceful shutdown
        let mut attempts = 0;
        let max_attempts = 10; // 5 seconds (500ms * 10)

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            attempts += 1;

            // Check if process still exists
            let still_running = std::process::Command::new("ps")
                .args(["-p", &pid.as_raw().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if !still_running {
                // Process no longer exists
                println!(
                    "{} {}",
                    "✓".bright_green().bold(),
                    "VPN disconnected gracefully".bright_green()
                );
                info!("OpenConnect process terminated gracefully");
                break;
            } else if attempts >= max_attempts {
                // Timeout, force kill (Step 4)
                warn!("Graceful shutdown timeout, force killing process");
                println!(
                    "{} {}",
                    "⚠".bright_yellow(),
                    "Process not responding, force killing...".bright_yellow()
                );

                let kill_result = std::process::Command::new("sudo")
                    .args(["kill", "-KILL", &pid.as_raw().to_string()])
                    .status();

                if let Err(e) = kill_result {
                    error!("Failed to send SIGKILL: {}", e);
                    return Err(AkonError::Vpn(VpnError::TerminationError));
                }

                // Wait a bit for SIGKILL to take effect
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                println!(
                    "{} {}",
                    "✓".bright_green().bold(),
                    "VPN disconnected (forced)".bright_green()
                );
                info!("OpenConnect process force-killed");
                break;
            }
            // Still running, continue waiting
        }
    } else {
        // Process not running, stale state (edge case from vpn-off-command.md)
        println!(
            "{} {}",
            "⚠".bright_yellow(),
            "VPN process no longer running (stale state)".dimmed()
        );
        info!(pid = pid.as_raw(), "Cleaning up stale connection state");
    }

    // Clean up state file (Step 5)
    fs::remove_file(&state_path).map_err(|e| {
        error!("Failed to remove state file: {}", e);
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to remove state file: {}", e),
        })
    })?;

    info!("State file cleaned up");
    debug!("Removed state file at {:?}", state_path);

    // Stop reconnection manager daemon if running
    stop_reconnection_manager_daemon();

    // Comprehensive cleanup: Terminate any orphaned OpenConnect processes
    println!(
        "{} {}",
        "🧹".bright_yellow(),
        "Cleaning up any orphaned OpenConnect processes...".bright_white()
    );

    info!("Starting comprehensive cleanup of orphaned processes");

    let result = cleanup_orphaned_processes();
    handle_cleanup_result(result, "run_vpn_off (after disconnect)");

    println!(
        "{} {}",
        "✓".bright_green(),
        "Disconnect complete".bright_green().bold()
    );

    Ok(())
}

/// Run the VPN status command
pub fn run_vpn_status() -> Result<(), AkonError> {
    use chrono::{DateTime, Utc};

    let state_path = state_file_path();

    if !state_path.exists() {
        println!(
            "{} {}",
            "●".bright_red(),
            "Status: Not connected".bright_white().bold()
        );
        std::process::exit(1);
    }

    // Read state file
    let state_content = fs::read_to_string(&state_path).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to read state file: {}", e),
        })
    })?;

    let state: serde_json::Value = serde_json::from_str(&state_content).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to parse state file: {}", e),
        })
    })?;

    // Check state from the state file
    let state_str = state.get("state").and_then(|s| s.as_str()).unwrap_or("");
    let is_reconnecting = state_str.contains("reconnecting") || state_str.contains("Reconnecting");
    let is_error = state_str.contains("Error") || state_str.contains("error");

    // T053: Check for Error state and suggest manual intervention
    if is_error {
        println!(
            "{} {}",
            "●".bright_red(),
            "Status: Error - Max reconnection attempts exceeded"
                .bright_red()
                .bold()
        );

        if let Some(error_msg) = state.get("error").and_then(|e| e.as_str()) {
            println!(
                "  {} {}",
                "Last error:".bright_white(),
                error_msg.bright_yellow()
            );
        }

        if let Some(attempts) = state.get("max_attempts").and_then(|a| a.as_u64()) {
            println!(
                "  {} Failed after {} reconnection attempts",
                "❌".bright_red(),
                attempts.to_string().bright_yellow()
            );
        }

        println!(
            "\n{} {}",
            "⚠".bright_yellow(),
            "Manual intervention required:".bright_white().bold()
        );
        println!(
            "  {} Run {} to disconnect",
            "1.".bright_yellow(),
            "akon vpn off".bright_cyan()
        );
        println!(
            "  {} Run {} to reconnect with reset",
            "2.".bright_yellow(),
            "akon vpn on --force".bright_cyan()
        );

        std::process::exit(3);
    }

    if is_reconnecting {
        // Display reconnecting status with attempt details
        let attempt = state.get("attempt").and_then(|a| a.as_u64()).unwrap_or(1);
        let max_attempts = state
            .get("max_attempts")
            .and_then(|m| m.as_u64())
            .unwrap_or(5);
        let next_retry_at = state.get("next_retry_at").and_then(|n| n.as_u64());

        println!(
            "{} {}",
            "●".bright_yellow(),
            "Status: Reconnecting".bright_yellow().bold()
        );
        println!(
            "  {} Attempt {} of {}",
            "🔄".bright_yellow(),
            attempt.to_string().bright_cyan(),
            max_attempts.to_string().bright_cyan()
        );

        if let Some(next_retry) = next_retry_at {
            let retry_time = DateTime::from_timestamp(next_retry as i64, 0)
                .map(|dt: DateTime<Utc>| dt.with_timezone(&chrono::Local))
                .map(|dt| dt.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            println!(
                "  {} Next retry at {}",
                "⏱".dimmed(),
                retry_time.bright_cyan()
            );
        }

        if let Some(ip) = state.get("last_ip") {
            println!(
                "  {} {}",
                "Last known IP:".dimmed(),
                ip.as_str().unwrap_or("unknown").bright_cyan()
            );
        }

        std::process::exit(1);
    }

    // Verify process is still running (Step 2 from vpn-status-command.md)
    // Note: openconnect runs as root, so we need to check via ps instead of kill signal
    let pid = state.get("pid").and_then(|p| p.as_u64());
    let process_running = if let Some(pid_num) = pid {
        // Use ps to check if process exists (works for processes owned by other users)
        std::process::Command::new("ps")
            .args(["-p", &pid_num.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        false
    };

    if !process_running {
        // Stale state
        println!(
            "{} {}",
            "●".bright_yellow(),
            "Status: Stale connection state".bright_yellow().bold()
        );
        println!(
            "  {} {}",
            "⚠".bright_yellow(),
            "Process no longer running".dimmed()
        );
        if let Some(ip) = state.get("ip") {
            println!(
                "  {} {}",
                "Last known IP:".dimmed(),
                ip.as_str().unwrap_or("unknown").bright_cyan()
            );
        }
        println!(
            "\n{} {} to clean up the stale state",
            "Run".dimmed(),
            "akon vpn off".bright_white().bold()
        );
        std::process::exit(2);
    }

    // Connected and process running
    println!(
        "{} {}",
        "●".bright_green(),
        "Status: Connected".bright_green().bold()
    );
    if let Some(ip) = state.get("ip") {
        println!(
            "  {} {}",
            "IP address:".bright_white(),
            ip.as_str().unwrap_or("unknown").bright_cyan().bold()
        );
    }
    if let Some(device) = state.get("device") {
        println!(
            "  {} {}",
            "Device:".bright_white(),
            device.as_str().unwrap_or("unknown").bright_cyan()
        );
    }
    if let Some(pid_num) = pid {
        println!(
            "  {} {}",
            "Process ID:".bright_white(),
            pid_num.to_string().bright_yellow()
        );
    }

    // Calculate and display duration
    if let Some(connected_at_str) = state.get("connected_at").and_then(|v| v.as_str()) {
        if let Ok(connected_at) = connected_at_str.parse::<DateTime<Utc>>() {
            let now = Utc::now();
            let duration = now.signed_duration_since(connected_at);

            let duration_str = if duration.num_days() > 0 {
                format!("{} days", duration.num_days())
            } else if duration.num_hours() > 0 {
                format!("{} hours", duration.num_hours())
            } else if duration.num_minutes() > 0 {
                format!("{} minutes", duration.num_minutes())
            } else {
                format!("{} seconds", duration.num_seconds())
            };

            println!(
                "  {} {}",
                "Duration:".bright_white(),
                duration_str.bright_magenta()
            );
            println!(
                "  {} {}",
                "Connected at:".bright_white(),
                connected_at
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string()
                    .dimmed()
            );
        }
    }

    Ok(())
}
