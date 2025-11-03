//! VPN connection management commands
//!
//! CLI-based OpenConnect integration using process delegation

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::load_config;
use akon_core::error::{AkonError, VpnError};
use akon_core::vpn::{CliConnector, ConnectionEvent};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

/// State file for tracking VPN connection
fn state_file_path() -> PathBuf {
    PathBuf::from("/tmp/akon_vpn_state.json")
}

/// Print actionable suggestions based on VPN error type
fn print_error_suggestions(error: &VpnError) {
    match error {
        VpnError::AuthenticationFailed => {
            eprintln!("\n{} {}", "💡".bright_yellow(), "Suggestions:".bright_white().bold());
            eprintln!("   {} Verify your PIN is correct", "•".bright_blue());
            eprintln!("   {} Check if your TOTP secret is valid", "•".bright_blue());
            eprintln!("   {} Run {} to reconfigure credentials", "•".bright_blue(), "akon setup".bright_cyan());
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
        VpnError::ConnectionFailed { reason } if reason.contains("TUN") || reason.contains("sudo") => {
            eprintln!("\n💡 Suggestions:");
            eprintln!("   • VPN requires root privileges to create TUN device");
            eprintln!("   • Run with: sudo akon vpn on");
            eprintln!("   • Ensure the 'tun' kernel module is loaded");
            eprintln!("   • Check: lsmod | grep tun");
        }
        VpnError::ProcessSpawnError { .. } => {
            eprintln!("\n{} {}", "💡".bright_yellow(), "Suggestions:".bright_white().bold());
            eprintln!("   {} OpenConnect may not be installed", "•".bright_blue());
            eprintln!("   {} Install with: {}", "•".bright_blue(), "sudo apt install openconnect".bright_cyan());
            eprintln!("   {} Or for RHEL/Fedora: {}", "•".bright_blue(), "sudo dnf install openconnect".bright_cyan());
            eprintln!("   {} Verify installation: {}", "•".bright_blue(), "which openconnect".bright_cyan());
        }
        VpnError::ConnectionFailed { reason } if reason.contains("Permission denied") => {
            eprintln!("\n{} {}", "💡".bright_yellow(), "Suggestions:".bright_white().bold());
            eprintln!("   {} This command requires elevated privileges", "•".bright_blue());
            eprintln!("   {} Run with: {}", "•".bright_blue(), "sudo akon vpn on".bright_cyan());
        }
        _ => {
            // Generic suggestions for other errors
            eprintln!("\n{} {}", "💡".bright_yellow(), "Suggestions:".bright_white().bold());
            eprintln!("   {} Check system logs: {}", "•".bright_blue(), "journalctl -xe".bright_cyan());
            eprintln!("   {} Verify configuration: {}", "•".bright_blue(), "cat ~/.config/akon/config.toml".bright_cyan());
            eprintln!("   {} Try reconnecting: {}", "•".bright_blue(), "akon vpn on".bright_cyan());
        }
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
                            // Force reconnection - disconnect first
                            info!("Force flag set, disconnecting existing connection (PID: {})", pid);
                            println!(
                                "{} {}",
                                "🔄".bright_yellow(),
                                "Force reconnection requested - disconnecting existing connection...".bright_yellow()
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

                            // Clean up state
                            let _ = fs::remove_file(&state_path);
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
    let config = load_config()?;
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

    // Monitor events with 60-second timeout
    let timeout_duration = tokio::time::Duration::from_secs(60);
    let timeout_result = tokio::time::timeout(timeout_duration, async {
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
    })
    .await;

    match timeout_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            error!("Connection timeout after 60 seconds");
            eprintln!(
                "{} {}",
                "❌".bright_red(),
                "Connection timeout after 60 seconds".bright_red().bold()
            );
            Err(AkonError::Vpn(VpnError::ConnectionTimeout { seconds: 60 }))
        }
    }
}

/// Run the VPN off command
pub async fn run_vpn_off() -> Result<(), AkonError> {
    use nix::unistd::Pid;

    // Load state file
    let state_path = state_file_path();

    if !state_path.exists() {
        println!("No active VPN connection found");
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
                println!("{} {}", "✓".bright_green().bold(), "VPN disconnected gracefully".bright_green());
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
        println!("  {} {}", "⚠".bright_yellow(), "Process no longer running".dimmed());
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
