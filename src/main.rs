//! akon - OTP-Integrated VPN CLI Tool
//!
//! A secure command-line tool for managing VPN connections with
//! automatic TOTP authentication using GNOME Keyring storage.

use akon_core::{error::AkonError, init_logging};
use clap::{Parser, Subcommand};

mod cli;
mod daemon;

#[derive(Parser)]
#[command(name = "akon")]
#[command(about = "VPN automatic authentication tool")]
#[command(version = "1.0.0")]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Setup VPN credentials securely
    ///
    /// Interactive wizard to configure VPN connection settings and credentials.
    ///
    /// CONFIGURATION FIELDS:
    ///
    /// • Server: VPN server hostname or IP address (e.g., vpn.example.com)
    ///
    /// • Username: Your VPN account username
    ///
    /// • PIN: Numeric PIN for authentication (stored securely in keyring)
    ///
    /// • TOTP Secret: Base32-encoded secret for generating time-based one-time passwords
    ///   (stored securely in keyring)
    ///
    /// • Protocol: VPN protocol type
    ///   - anyconnect: Cisco AnyConnect SSL VPN
    ///   - gp: Palo Alto Networks GlobalProtect
    ///   - nc: Juniper Network Connect
    ///   - pulse: Pulse Connect Secure
    ///   - f5: F5 Big-IP SSL VPN (default)
    ///   - fortinet: Fortinet FortiGate SSL VPN
    ///   - array: Array Networks SSL VPN
    ///
    /// • Timeout: Connection timeout in seconds (default: 30)
    ///
    /// • No DTLS: Disable DTLS and use only TCP/TLS (default: false)
    ///   Use this if DTLS is blocked by firewall or causes connection issues
    ///
    /// • Lazy Mode: When enabled, running 'akon' without arguments automatically
    ///   connects to VPN. When disabled, you must use 'akon vpn on' (default: false)
    ///
    /// STORAGE:
    ///
    /// • Config file: ~/.config/akon/config.toml (non-sensitive settings)
    /// • Credentials: GNOME Keyring (PIN and TOTP secret, encrypted)
    ///
    /// EXAMPLES:
    ///
    /// # Run setup wizard
    /// akon setup
    ///
    /// # View this help
    /// akon setup --help
    Setup,
    /// Manage VPN connection (on/off/status)
    Vpn {
        #[command(subcommand)]
        action: VpnCommands,
    },
    /// Generate OTP token for manual use
    GetPassword,
}

#[derive(Subcommand)]
enum VpnCommands {
    /// Connect to VPN
    On {
        /// Force reconnection (disconnects existing connection and resets state)
        #[arg(short, long)]
        force: bool,
    },
    /// Disconnect from VPN
    Off,
    /// Show VPN connection status
    Status,
}

#[tokio::main]
async fn main() {
    // Check if this is an internal daemon invocation (before parsing CLI)
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "__internal_reconnection_daemon" {
        // This is a daemon process invocation
        handle_daemon_invocation(args).await;
        return;
    }

    // Initialize logging
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        std::process::exit(2);
    }

    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Setup) => cli::setup::run_setup(),
        Some(Commands::Vpn { action }) => match action {
            VpnCommands::On { force } => cli::vpn::run_vpn_on(force).await,
            VpnCommands::Off => cli::vpn::run_vpn_off().await,
            VpnCommands::Status => cli::vpn::run_vpn_status(),
        },
        Some(Commands::GetPassword) => cli::get_password::run_get_password(),
        None => {
            // No command provided - check for lazy mode
            use akon_core::config::toml_config::load_config;
            match load_config() {
                Ok(config) if config.lazy_mode => {
                    // Lazy mode enabled - run vpn on
                    cli::vpn::run_vpn_on(false).await
                }
                Ok(_) => {
                    // Config exists but lazy mode disabled - show help
                    use clap::CommandFactory;
                    Cli::command().print_help().unwrap();
                    std::process::exit(2);
                }
                Err(_) => {
                    // No config - show help
                    use clap::CommandFactory;
                    Cli::command().print_help().unwrap();
                    std::process::exit(2);
                }
            }
        }
    };

    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            let exit_code = match e {
                // Configuration errors (exit code 2)
                AkonError::Config(_) | AkonError::Toml(_) | AkonError::TomlSerialize(_) => 2,
                // Keyring errors (exit code 2 for configuration/setup issues)
                AkonError::Keyring(_) => 2,
                // VPN errors - distinguish between auth/network vs config
                AkonError::Vpn(ref vpn_error) => match vpn_error {
                    akon_core::error::VpnError::ConnectionFailed { .. } => 1,
                    akon_core::error::VpnError::AuthenticationFailed => 1,
                    akon_core::error::VpnError::NetworkError { .. } => 1,
                    akon_core::error::VpnError::InvalidStateTransition => 1,
                    akon_core::error::VpnError::OpenConnectError { .. } => 1,
                    akon_core::error::VpnError::ProcessSpawnError { .. } => 1,
                    akon_core::error::VpnError::ConnectionTimeout { .. } => 1,
                    akon_core::error::VpnError::TerminationError => 1,
                    akon_core::error::VpnError::ParseError { .. } => 1,
                },
                // OTP errors (exit code 2 - configuration/setup)
                AkonError::Otp(_) => 2,
                // IO errors (exit code 1 - runtime)
                AkonError::Io(_) => 1,
            };

            eprintln!("{}", e);
            std::process::exit(exit_code);
        }
    }
}

/// Handle internal daemon invocation
/// This function is called when the process is spawned as a daemon
async fn handle_daemon_invocation(args: Vec<String>) {
    // Initialize logging for daemon
    if let Err(e) = init_logging() {
        eprintln!("Daemon: Failed to initialize logging: {}", e);
        std::process::exit(2);
    }

    // Parse policy and config from arguments
    let policy_json = &args[2];
    let config_json = &args[3];

    let policy: akon_core::vpn::reconnection::ReconnectionPolicy = match serde_json::from_str(policy_json) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Daemon: Failed to parse reconnection policy: {}", e);
            std::process::exit(2);
        }
    };

    let config: akon_core::config::VpnConfig = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Daemon: Failed to parse VPN config: {}", e);
            std::process::exit(2);
        }
    };

    // Run the reconnection manager
    if let Err(e) = cli::vpn::run_reconnection_manager_daemon(policy, config).await {
        eprintln!("Daemon: Reconnection manager error: {}", e);
        std::process::exit(1);
    }
}
