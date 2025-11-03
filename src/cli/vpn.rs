//! VPN connection management commands
//!
//! CLI-based OpenConnect integration using process delegation

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::load_config;
use akon_core::error::{AkonError, VpnError};
use akon_core::vpn::{CliConnector, ConnectionEvent};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, error, info};

/// State file for tracking VPN connection
fn state_file_path() -> PathBuf {
    PathBuf::from("/tmp/akon_vpn_state.json")
}

/// Run the VPN on command using CLI process delegation
pub async fn run_vpn_on() -> Result<(), AkonError> {
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
    println!("Connecting to VPN server: {}...", config.server);
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
                    println!("🔐 {}", message);
                    info!(phase = "authentication", message = %message, "Authentication in progress");
                }
                ConnectionEvent::F5SessionEstablished { .. } => {
                    println!("✓ Secure F5 session established");
                    info!(phase = "session", "F5 session established");
                }
                ConnectionEvent::TunConfigured { device, ip } => {
                    println!("✓ TUN device configured: {} ({})", device, ip);
                    info!(device = %device, ip = %ip, "TUN device configured");
                }
                ConnectionEvent::Connected { ip, device } => {
                    println!("✓ VPN connection established");
                    println!("  IP address: {}", ip);
                    println!("  Device: {}", device);
                    info!(ip = %ip, device = %device, "VPN connection fully established");

                    // Save state for status command
                    let state = serde_json::json!({
                        "ip": ip.to_string(),
                        "device": device,
                        "connected_at": chrono::Utc::now().to_rfc3339(),
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
                    eprintln!("Error: {}", kind);
                    if !raw_output.is_empty() {
                        eprintln!("Details: {}", raw_output);
                    }
                    return Err(AkonError::Vpn(kind));
                }
                ConnectionEvent::Disconnected { reason } => {
                    info!("VPN disconnected: {:?}", reason);
                    println!("VPN disconnected: {:?}", reason);
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
            eprintln!("Error: Connection timeout after 60 seconds");
            Err(AkonError::Vpn(VpnError::ConnectionTimeout { seconds: 60 }))
        }
    }
}

/// Run the VPN off command
pub async fn run_vpn_off() -> Result<(), AkonError> {
    // Load state file
    let state_path = state_file_path();

    if !state_path.exists() {
        println!("No active VPN connection found");
        return Ok(());
    }

    // For now, just remove the state file
    // Full implementation would load PID and terminate the process
    fs::remove_file(&state_path).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to remove state file: {}", e),
        })
    })?;

    println!("VPN connection state cleared");
    println!("Note: You may need to manually kill the openconnect process");
    Ok(())
}

/// Run the VPN status command
pub fn run_vpn_status() -> Result<(), AkonError> {
    let state_path = state_file_path();

    if !state_path.exists() {
        println!("Status: Not connected");
        return Ok(());
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

    println!("Status: Connected");
    if let Some(ip) = state.get("ip") {
        println!("  IP address: {}", ip.as_str().unwrap_or("unknown"));
    }
    if let Some(device) = state.get("device") {
        println!("  Device: {}", device.as_str().unwrap_or("unknown"));
    }
    if let Some(connected_at) = state.get("connected_at") {
        println!("  Connected at: {}", connected_at.as_str().unwrap_or("unknown"));
    }

    Ok(())
}
