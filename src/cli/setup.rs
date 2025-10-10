//! Setup command implementation
//!
//! Interactive command for first-time VPN configuration with secure credential storage.

use akon_core::{
    auth::keyring,
    config::{toml_config, VpnConfig},
    error::AkonError,
    types::OtpSecret,
};
use std::io::{self, Write};

/// Run the setup command
pub fn run_setup() -> Result<(), AkonError> {
    println!("🔐 akon VPN Setup");
    println!("=================");
    println!();
    println!("This will configure your VPN connection securely.");
    println!("Credentials will be stored in your system keyring.");
    println!("Configuration will be saved to ~/.config/akon/config.toml");
    println!();

    // Check if already configured
    if let Ok(true) = toml_config::config_exists() {
        println!("⚠️  Existing configuration detected.");
        if !prompt_yes_no("Overwrite existing setup? (y/N)", false)? {
            println!("Setup cancelled.");
            return Ok(());
        }
        println!();
    }

    // Check keyring availability
    check_keyring_availability()?;

    // Collect configuration interactively
    let config = collect_vpn_config()?;
    let otp_secret = collect_otp_secret()?;

    // Validate configuration
    config.validate().map_err(|e| {
        AkonError::Config(akon_core::error::ConfigError::ValidationError {
            message: format!("Configuration validation failed: {}", e),
        })
    })?;

    // Validate OTP secret
    otp_secret
        .validate_base32()
        .map_err(|e| AkonError::Otp(e))?;

    // Save configuration
    println!();
    println!("💾 Saving configuration...");

    // Save config to TOML file
    toml_config::save_config(&config)?;

    // Store OTP secret in keyring
    keyring::store_otp_secret(&config.username, otp_secret.expose())?;

    println!("✅ Setup complete!");
    println!();
    println!("You can now use:");
    println!("  akon vpn on     - Connect to VPN");
    println!("  akon vpn off    - Disconnect from VPN");
    println!("  akon get-password - Generate OTP token manually");

    Ok(())
}

/// Check if the keyring is available
fn check_keyring_availability() -> Result<(), AkonError> {
    // Try to create a test entry to check keyring availability
    match keyring::store_otp_secret("__akon_test__", "test") {
        Ok(_) => {
            // Clean up test entry
            let _ = keyring::delete_otp_secret("__akon_test__");
            Ok(())
        }
        Err(AkonError::Keyring(_)) => {
            println!("❌ Keyring is not available or locked.");
            println!("Please ensure your system keyring is unlocked and available.");
            println!("On GNOME systems, this is usually handled automatically.");
            Err(AkonError::Keyring(
                akon_core::error::KeyringError::ServiceUnavailable,
            ))
        }
        Err(e) => Err(e),
    }
}

/// Collect VPN configuration interactively
fn collect_vpn_config() -> Result<VpnConfig, AkonError> {
    println!("VPN Configuration:");
    println!("-----------------");

    let server = prompt_required("VPN Server (hostname or IP)", "vpn.example.com")?;
    let port: u16 = prompt_required("VPN Port", "443")?.parse().map_err(|_| {
        AkonError::Config(akon_core::error::ConfigError::ValidationError {
            message: "Invalid port number".to_string(),
        })
    })?;

    let username = prompt_required("Username", "")?;
    let realm = prompt_optional("Realm (optional)", "")?;
    let timeout: Option<u32> = prompt_optional("Connection timeout in seconds (optional)", "30")?
        .parse()
        .ok();

    let realm = if realm.trim().is_empty() {
        None
    } else {
        Some(realm.trim().to_string())
    };

    Ok(VpnConfig {
        server,
        port,
        username,
        realm,
        timeout,
    })
}

/// Collect OTP secret interactively
fn collect_otp_secret() -> Result<OtpSecret, AkonError> {
    println!();
    println!("OTP Configuration:");
    println!("-----------------");

    println!("Enter your TOTP secret (Base32-encoded, e.g., JBSWY3DPEHPK3PXP)");
    println!("This will be stored securely in your system keyring.");
    println!();

    loop {
        let secret = prompt_password("TOTP Secret")?;

        if secret.trim().is_empty() {
            println!("❌ Secret cannot be empty. Please try again.");
            continue;
        }

        let otp_secret = OtpSecret::new(secret);

        match otp_secret.validate_base32() {
            Ok(_) => return Ok(otp_secret),
            Err(_) => {
                println!("❌ Invalid Base32 format. Please check your secret and try again.");
                println!("   Valid characters: A-Z, 2-7, =, /");
                continue;
            }
        }
    }
}

/// Prompt for a required value with default
fn prompt_required(prompt: &str, default: &str) -> Result<String, AkonError> {
    let prompt_text = if default.is_empty() {
        format!("{}: ", prompt)
    } else {
        format!("{} [{}]: ", prompt, default)
    };

    loop {
        let input = prompt_input(&prompt_text)?;

        if input.trim().is_empty() {
            if !default.is_empty() {
                return Ok(default.to_string());
            }
            println!("❌ This field is required. Please enter a value.");
            continue;
        }

        return Ok(input.trim().to_string());
    }
}

/// Prompt for an optional value
fn prompt_optional(prompt: &str, default: &str) -> Result<String, AkonError> {
    let prompt_text = format!("{} [{}]: ", prompt, default);
    let input = prompt_input(&prompt_text)?;

    if input.trim().is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.trim().to_string())
    }
}

/// Prompt for a password (hidden input)
fn prompt_password(prompt: &str) -> Result<String, AkonError> {
    let prompt_text = format!("{}: ", prompt);
    prompt_input(&prompt_text)
}

/// Prompt for yes/no with default
fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool, AkonError> {
    let default_indicator = if default_yes { "[Y/n]" } else { "[y/N]" };
    let prompt_text = format!("{} {}: ", prompt, default_indicator);

    loop {
        let input = prompt_input(&prompt_text)?.to_lowercase();

        match input.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            "" => return Ok(default_yes),
            _ => {
                println!("Please enter 'y' for yes or 'n' for no.");
                continue;
            }
        }
    }
}

/// Low-level input prompting
fn prompt_input(prompt: &str) -> Result<String, AkonError> {
    print!("{}", prompt);
    io::stdout().flush().map_err(AkonError::Io)?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(AkonError::Io)?;

    Ok(input.trim_end().to_string())
}
