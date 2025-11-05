//! Setup command implementation
//!
//! Interactive command for first-time VPN configuration with secure credential storage.

use akon_core::{
    auth::keyring,
    config::{toml_config, VpnConfig},
    error::AkonError,
    types::OtpSecret,
};
use colored::Colorize;
use std::io::{self, Write};

/// Run the setup command
pub fn run_setup() -> Result<(), AkonError> {
    println!(
        "{} {}",
        "🔐".bright_magenta(),
        "akon VPN Setup".bright_white().bold()
    );
    println!("{}", "=================".bright_white());
    println!();
    println!(
        "{}",
        "This will configure your VPN connection securely.".bright_white()
    );
    println!(
        "{}",
        "Credentials will be stored in your system keyring.".dimmed()
    );
    println!(
        "{}",
        "Configuration will be saved to ~/.config/akon/config.toml".dimmed()
    );
    println!();

    // Check if already configured
    if let Ok(true) = toml_config::config_exists() {
        println!(
            "{} {}",
            "⚠".bright_yellow(),
            "Existing configuration detected.".bright_yellow()
        );
        if !prompt_yes_no("Overwrite existing setup? (y/N)", false)? {
            println!("{}", "Setup cancelled.".dimmed());
            return Ok(());
        }
        println!();
    }

    // Check keyring availability
    check_keyring_availability()?;

    // Collect configuration interactively
    let config = collect_vpn_config()?;
    let otp_secret = collect_otp_secret()?;
    let reconnection_policy = collect_reconnection_config()?;

    // Validate configuration
    config.validate().map_err(|e| {
        AkonError::Config(akon_core::error::ConfigError::ValidationError {
            message: format!("Configuration validation failed: {}", e),
        })
    })?;

    // Validate OTP secret
    otp_secret.validate_base32().map_err(AkonError::Otp)?;

    // Save configuration
    println!();
    println!(
        "{} {}",
        "💾".bright_cyan(),
        "Saving configuration...".bright_white()
    );

    // Save config to TOML file with reconnection policy
    toml_config::save_config_with_reconnection(&config, reconnection_policy.as_ref())?;

    // Store OTP secret in keyring
    keyring::store_otp_secret(&config.username, otp_secret.expose())?;

    println!(
        "{} {}",
        "✅".bright_green(),
        "Setup complete!".bright_green().bold()
    );
    println!();
    println!("{}", "You can now use:".bright_white());
    println!("  {} - Connect to VPN", "akon vpn on".bright_cyan());
    println!("  {} - Disconnect from VPN", "akon vpn off".bright_cyan());
    println!(
        "  {} - Generate OTP token manually",
        "akon get-password".bright_cyan()
    );

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
    println!("{}", "VPN Configuration:".bright_white().bold());
    println!("{}", "-----------------".bright_white());

    let server = prompt_required("VPN Server (hostname or IP)", "vpn.example.com")?;
    let username = prompt_required("Username", "")?;

    println!();
    println!("Protocol selection:");
    println!("  1. AnyConnect (Cisco)");
    println!("  2. GlobalProtect (Palo Alto)");
    println!("  3. Network Connect (Juniper)");
    println!("  4. Pulse Connect Secure");
    println!("  5. F5 Big-IP [default]");
    println!("  6. Fortinet FortiGate");
    println!("  7. Array Networks");

    let protocol_choice = prompt_optional("Select protocol (1-7)", "5")?;
    let protocol = match protocol_choice.trim() {
        "1" => akon_core::config::VpnProtocol::AnyConnect,
        "2" => akon_core::config::VpnProtocol::GlobalProtect,
        "3" => akon_core::config::VpnProtocol::NC,
        "4" => akon_core::config::VpnProtocol::Pulse,
        "6" => akon_core::config::VpnProtocol::Fortinet,
        "7" => akon_core::config::VpnProtocol::Array,
        _ => akon_core::config::VpnProtocol::F5, // Default
    };

    let timeout: Option<u32> = prompt_optional("Connection timeout in seconds (optional)", "30")?
        .parse()
        .ok();

    let no_dtls_input = prompt_optional("Disable DTLS (use TCP only)? (y/N)", "n")?;
    let no_dtls = matches!(no_dtls_input.trim().to_lowercase().as_str(), "y" | "yes");

    let lazy_mode_input = prompt_optional(
        "Enable lazy mode (connect VPN when running akon without arguments)? (y/N)",
        "n",
    )?;
    let lazy_mode = matches!(lazy_mode_input.trim().to_lowercase().as_str(), "y" | "yes");

    Ok(VpnConfig {
        server,
        username,
        protocol,
        timeout,
        no_dtls,
        lazy_mode,
    })
}

/// Collect reconnection configuration interactively
fn collect_reconnection_config() -> Result<Option<akon_core::vpn::reconnection::ReconnectionPolicy>, AkonError> {
    use akon_core::vpn::reconnection::ReconnectionPolicy;

    println!();
    println!("Reconnection Configuration (Optional):");
    println!("-------------------------------------");
    println!("Configure automatic reconnection when network interruptions occur.");
    println!();

    if !prompt_yes_no("Configure automatic reconnection? (Y/n)", true)? {
        println!("{}", "Skipping reconnection config - defaults will be used if needed.".dimmed());
        return Ok(None);
    }

    println!();
    println!("{}", "Basic Settings:".bright_white().bold());
    println!();

    // Health check endpoint (required for reconnection)
    println!("Enter the health check endpoint (HTTP/HTTPS URL to verify connectivity)");
    println!("{}", "Example: https://vpn-gateway.example.com/health".dimmed());
    let health_check_endpoint = prompt_required("Health Check Endpoint", "https://www.google.com")?;

    // Validate URL
    if !health_check_endpoint.starts_with("http://") && !health_check_endpoint.starts_with("https://") {
        return Err(AkonError::Config(akon_core::error::ConfigError::ValidationError {
            message: "Health check endpoint must be an HTTP or HTTPS URL".to_string(),
        }));
    }

    println!();
    if !prompt_yes_no("Configure advanced reconnection settings? (y/N)", false)? {
        // Use defaults for everything else
        let policy = ReconnectionPolicy {
            max_attempts: 5,
            base_interval_secs: 5,
            backoff_multiplier: 2,
            max_interval_secs: 60,
            consecutive_failures_threshold: 2,
            health_check_interval_secs: 60,
            health_check_endpoint,
        };

        policy.validate().map_err(|e| {
            AkonError::Config(akon_core::error::ConfigError::ValidationError {
                message: format!("Reconnection policy validation failed: {}", e),
            })
        })?;

        return Ok(Some(policy));
    }

    println!();
    println!("{}", "Advanced Settings:".bright_white().bold());
    println!();

    // Max attempts
    println!("Maximum reconnection attempts before requiring manual intervention (1-20)");
    let max_attempts_str = prompt_optional("Max Attempts", "5")?;
    let max_attempts = max_attempts_str.parse::<u32>().unwrap_or(5);

    // Base interval
    println!();
    println!("Base interval in seconds for exponential backoff (1-300)");
    let base_interval_str = prompt_optional("Base Interval (seconds)", "5")?;
    let base_interval_secs = base_interval_str.parse::<u32>().unwrap_or(5);

    // Backoff multiplier
    println!();
    println!("Exponential backoff multiplier (1-10)");
    println!("{}", "Intervals will be: base × multiplier^(attempt-1)".dimmed());
    let backoff_multiplier_str = prompt_optional("Backoff Multiplier", "2")?;
    let backoff_multiplier = backoff_multiplier_str.parse::<u32>().unwrap_or(2);

    // Max interval
    println!();
    println!("Maximum interval in seconds (cap for exponential growth)");
    let max_interval_str = prompt_optional("Max Interval (seconds)", "60")?;
    let max_interval_secs = max_interval_str.parse::<u32>().unwrap_or(60);

    // Consecutive failures
    println!();
    println!("Number of consecutive health check failures before triggering reconnection (1-10)");
    let consecutive_failures_str = prompt_optional("Consecutive Failures Threshold", "2")?;
    let consecutive_failures_threshold = consecutive_failures_str.parse::<u32>().unwrap_or(2);

    // Health check interval
    println!();
    println!("Health check interval in seconds (10-3600)");
    let health_check_interval_str = prompt_optional("Health Check Interval (seconds)", "60")?;
    let health_check_interval_secs = health_check_interval_str.parse::<u64>().unwrap_or(60);

    let policy = ReconnectionPolicy {
        max_attempts,
        base_interval_secs,
        backoff_multiplier,
        max_interval_secs,
        consecutive_failures_threshold,
        health_check_interval_secs,
        health_check_endpoint,
    };

    // Validate the policy
    policy.validate().map_err(|e| {
        AkonError::Config(akon_core::error::ConfigError::ValidationError {
            message: format!("Reconnection policy validation failed: {}", e),
        })
    })?;

    println!();
    println!("{} {}", "✓".bright_green(), "Reconnection configuration validated".bright_green());

    Ok(Some(policy))
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
