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
#[command(about = "OTP-Integrated VPN CLI with secure credential management")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Setup VPN credentials securely
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
    On,
    /// Disconnect from VPN
    Off,
    /// Show VPN connection status
    Status,
}

fn main() {
    // Initialize logging
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        std::process::exit(2);
    }

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Setup => cli::setup::run_setup(),
        Commands::Vpn { action } => match action {
            VpnCommands::On => cli::vpn::run_vpn_on(),
            VpnCommands::Off => cli::vpn::run_vpn_off(),
            VpnCommands::Status => cli::vpn::run_vpn_status(),
        },
        Commands::GetPassword => cli::get_password::run_get_password(),
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
