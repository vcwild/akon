//! Get password command implementation
//!
//! This module implements the `akon get-password` command that generates
//! and outputs complete VPN passwords (PIN + OTP) for manual use.

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::load_config;
use akon_core::error::AkonError;

/// Run the get-password command
///
/// Outputs the complete VPN password (PIN + OTP) to stdout for machine-parsable usage.
/// Errors are sent to stderr. No additional formatting or text.
pub fn run_get_password() -> Result<(), AkonError> {
    // Load configuration to get username
    let config = load_config()?;

    // Generate complete password (PIN + OTP) from keyring credentials
    let password = generate_password(&config.username)?;

    // Output only the password to stdout (machine-parsable)
    println!("{}", password.expose());

    Ok(())
}
