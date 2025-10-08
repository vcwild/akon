//! akon - OTP-Integrated VPN CLI Tool
//!
//! A secure command-line tool for managing VPN connections with
//! automatic TOTP authentication using GNOME Keyring storage.

use clap::{Parser, Subcommand};
use akon_core::{error::AkonError, init_logging};

mod cli;

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

fn main() -> Result<(), AkonError> {
    // Initialize logging
    init_logging().map_err(|e| AkonError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("Failed to initialize logging: {}", e),
    )))?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Setup => {
            cli::setup::run_setup()
        }
        Commands::Vpn { action } => match action {
            VpnCommands::On => {
                println!("VPN on command not implemented yet");
                Ok(())
            }
            VpnCommands::Off => {
                println!("VPN off command not implemented yet");
                Ok(())
            }
            VpnCommands::Status => {
                println!("VPN status command not implemented yet");
                Ok(())
            }
        },
        Commands::GetPassword => {
            println!("Get password command not implemented yet");
            Ok(())
        }
    }
}
