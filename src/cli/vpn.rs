//! VPN connection management commands
//!
//! This module implements the VPN on/off/status commands that interact
//! with the daemon process for connection management.

use std::thread;
use std::time::Duration;

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::load_config;
use akon_core::error::{AkonError, VpnError};
use akon_core::vpn::state::{ConnectionMetadata, ConnectionState, SharedConnectionState};
use tracing::{error, info};

use crate::daemon::ipc::{get_default_socket_path, IpcClient};
use crate::daemon::process::{get_default_pid_file, DaemonProcess};

/// Run the VPN on command
pub fn run_vpn_on() -> Result<(), AkonError> {
    // Check if daemon is already running
    let daemon = DaemonProcess::new(get_default_pid_file());
    if daemon.is_running()? {
        println!("VPN daemon is already running");
        return Ok(());
    }

    // Load configuration
    let config = load_config()?;
    println!("Loaded configuration for server: {}", config.server);

    // Generate complete VPN password (PIN + OTP)
    let password = generate_password(&config.username)?;
    println!("Generated VPN password from keyring credentials");

    // Create shared connection state
    let connection_state = SharedConnectionState::new();

    // Spawn daemon process
    println!("Starting VPN daemon...");
    match unsafe { libc::fork() } {
        0 => {
            // Child process (daemon)
            run_daemon(&config, &password, &connection_state)?;
            std::process::exit(0);
        }
        pid if pid > 0 => {
            // Parent process
            println!("Daemon started with PID {}", pid);

            // Wait for connection to be established or fail
            wait_for_connection(&connection_state)?;
            println!("VPN connection established successfully");
            Ok(())
        }
        _ => Err(AkonError::Vpn(VpnError::ConnectionFailed {
            reason: "Failed to fork daemon process".to_string(),
        })),
    }
}

/// Run the VPN off command
pub fn run_vpn_off() -> Result<(), AkonError> {
    let client = IpcClient::new(get_default_socket_path());

    match client.disconnect() {
        Ok(()) => {
            println!("VPN disconnection requested");
            Ok(())
        }
        Err(e) => {
            // If IPC fails, try to stop daemon directly
            let daemon = DaemonProcess::new(get_default_pid_file());
            if daemon.is_running()? {
                daemon.stop()?;
                println!("VPN daemon stopped");
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

/// Run the VPN status command
pub fn run_vpn_status() -> Result<(), AkonError> {
    let daemon = DaemonProcess::new(get_default_pid_file());

    if !daemon.is_running()? {
        println!("VPN Status: Disconnected");
        println!("No VPN daemon is running");
        return Ok(());
    }

    // Daemon is running, try to query its status via IPC
    let socket_path = get_default_socket_path();
    let client = IpcClient::new(socket_path);

    match client.get_status() {
        Ok(state) => match state {
            ConnectionState::Disconnected => {
                println!("VPN Status: Disconnected");
            }
            ConnectionState::Connecting => {
                println!("VPN Status: Connecting...");
            }
            ConnectionState::Connected(ref metadata) => {
                println!("VPN Status: Connected");
                println!("Server: {}", metadata.server);
                println!("Username: {}", metadata.username);
                println!("Uptime: {}", metadata.uptime_display());
                println!("Daemon PID: {}", daemon.get_pid()?);
            }
            ConnectionState::Disconnecting => {
                println!("VPN Status: Disconnecting...");
            }
            ConnectionState::Error(ref reason) => {
                println!("VPN Status: Error - {}", reason);
            }
        },
        Err(_) => {
            // IPC communication failed, but daemon is running
            println!("VPN Status: Unknown (daemon running but IPC unavailable)");
            println!("Daemon PID: {}", daemon.get_pid()?);
        }
    }

    Ok(())
}

/// Run the daemon process
fn run_daemon(
    config: &akon_core::config::VpnConfig,
    password: &akon_core::types::VpnPassword,
    connection_state: &SharedConnectionState,
) -> Result<(), AkonError> {
    use akon_core::vpn::openconnect::OpenConnectConnection;

    // Daemonize the process FIRST - before initializing any resources
    let daemon = DaemonProcess::new(get_default_pid_file());
    daemon.daemonize()?;

    // IMPORTANT: After daemonize(), we're in the child process with clean state
    // Now it's safe to initialize OpenConnect and other resources

    // Initialize OpenConnect SSL library in the child process after fork
    // This is critical - SSL state doesn't survive fork()
    info!("Initializing OpenConnect SSL in child process");
    if let Err(e) = OpenConnectConnection::init_ssl() {
        let error_msg = format!("Failed to initialize OpenConnect SSL: {}", e);
        error!("{}", error_msg);
        connection_state.set_error(error_msg);
        return Err(e);
    }
    info!("OpenConnect SSL initialized successfully");

    // Set up IPC server
    let ipc_server =
        crate::daemon::ipc::IpcServer::new(get_default_socket_path(), connection_state.clone())?;

    // Start IPC server in a separate thread
    let ipc_handle = thread::spawn(move || {
        if let Err(e) = ipc_server.run() {
            eprintln!("IPC server error: {}", e);
        }
    });

    // Update state to connecting
    connection_state.set(ConnectionState::Connecting);

    // Create OpenConnect connection AFTER daemonizing
    // This ensures OpenConnect initializes in the child process with clean state
    info!("Creating OpenConnect connection...");
    let mut vpn_conn = match OpenConnectConnection::new() {
        Ok(conn) => {
            info!("OpenConnect connection created successfully");
            conn
        }
        Err(e) => {
            let error_msg = format!("Failed to initialize OpenConnect: {}", e);
            error!("{}", error_msg);
            connection_state.set_error(error_msg.clone());
            return Err(e);
        }
    };

    // Construct server URL - for F5 and other protocols, OpenConnect can work with just hostname
    // OpenConnect will add https:// and proper port based on protocol if not specified
    let server_url = if config.port == 443 {
        // For standard HTTPS port, use just the hostname - let OpenConnect handle it
        config.server.clone()
    } else {
        // For non-standard ports, include it
        format!("{}:{}", config.server, config.port)
    };

    // Connect to VPN (protocol is set inside connect now)
    match vpn_conn.connect(
        &server_url,
        &config.username,
        password.expose(),
        config.protocol.as_str(),
        config.no_dtls,
    ) {
        Ok(()) => {
            let metadata = ConnectionMetadata::new(config.server.clone(), config.username.clone());
            connection_state.set(ConnectionState::Connected(metadata));
        }
        Err(e) => {
            let error_msg = format!("VPN connection failed: {}", e);
            connection_state.set_error(error_msg.clone());
            return Err(e);
        }
    }

    // Run main loop
    match vpn_conn.run_mainloop() {
        Ok(()) => {
            connection_state.set_disconnected();
        }
        Err(e) => {
            let error_msg = format!("VPN main loop error: {}", e);
            connection_state.set_error(error_msg);
            return Err(e);
        }
    }

    // Wait for IPC server to finish
    let _ = ipc_handle.join();

    Ok(())
}

/// Wait for VPN connection to be established
fn wait_for_connection(connection_state: &SharedConnectionState) -> Result<(), AkonError> {
    let mut attempts = 0;
    let max_attempts = 15; // 15 seconds timeout
    let step = 5;

    while attempts < max_attempts {
        match connection_state.get() {
            ConnectionState::Connected(_) => return Ok(()),
            ConnectionState::Error(ref reason) => {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: reason.clone(),
                }));
            }
            _ => {
                thread::sleep(Duration::from_secs(step));
                attempts += step;
                println!("Waiting for VPN connection... ({}s)", attempts);
            }
        }
    }

    Err(AkonError::Vpn(VpnError::ConnectionFailed {
        reason: "Connection timeout".to_string(),
    }))
}
