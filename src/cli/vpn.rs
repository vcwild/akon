//! VPN connection management commands
//!
//! Single-threaded implementation for debugging OpenConnect integration.

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::load_config;
use akon_core::error::AkonError;
use tracing::{error, info};

/// Run the VPN on command (single-threaded for debugging)
pub fn run_vpn_on() -> Result<(), AkonError> {
    use akon_core::vpn::openconnect::OpenConnectConnection;

    // Load configuration
    let config = load_config()?;
    println!("Loaded configuration for server: {}", config.server);

    // Generate complete VPN password (PIN + OTP) from user's keyring
    // This works seamlessly whether running as user or with elevated privileges
    // because the config path is resolved correctly for the actual user
    let password = generate_password(&config.username)?;
    println!("Generated VPN password from keyring credentials");

    // Initialize OpenConnect SSL ONCE at startup (before creating vpninfo)
    info!("Initializing OpenConnect SSL library");
    OpenConnectConnection::init_ssl()?;
    println!("✓ OpenConnect SSL initialized");

    // Create OpenConnect connection with credentials - this creates vpninfo ONCE
    info!("Creating OpenConnect connection with credentials");
    let mut vpn_conn = OpenConnectConnection::new(&config.username, password.expose())
        .map_err(|e| {
            error!("Failed to create OpenConnect connection: {}", e);
            e
        })?;
    println!("✓ OpenConnect connection created");

    // Connect to VPN (session only, no TUN device or routing)
    info!("Connecting to VPN server: {}", config.server);
    println!("DEBUG: About to call vpn_conn.connect()");
    vpn_conn.connect(
        config.server.as_str(),
        config.protocol.as_str(),
    )?;
    println!("DEBUG: connect() returned successfully");

    info!("CSTP connection established, starting PPP negotiation...");
    println!("✓ CSTP connection established");
    println!("Completing PPP negotiation (you should see 'Configured as...' message)...");

    // Complete the connection - this runs mainloop which performs PPP negotiation
    // It will show "Configured as X.X.X.X" then fail on TUN setup (expected without root)
    info!("Completing connection (PPP negotiation in mainloop)...");
    vpn_conn.complete_connection()?;

    info!("Connection cycle completed");
    println!("✓ VPN session was established (see 'Configured as X.X.X.X' above)");
    println!("Note: TUN setup failed as expected (no root) - session valid but no routing");
    println!("VPN disconnected");
    Ok(())
}

/// Run the VPN off command (not implemented in single-threaded mode)
pub fn run_vpn_off() -> Result<(), AkonError> {
    println!("VPN off: Use Ctrl+C to stop the running VPN connection");
    println!("(Daemon mode not available in single-threaded debug mode)");
    Ok(())
}

/// Run the VPN status command (not implemented in single-threaded mode)
pub fn run_vpn_status() -> Result<(), AkonError> {
    println!("VPN status: Not available in single-threaded debug mode");
    println!("Check if 'akon vpn on' is running in your terminal");
    Ok(())
}
