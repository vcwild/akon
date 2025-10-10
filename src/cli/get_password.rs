//! Get password command implementation
//!
//! This module implements the `akon get-password` command that generates
//! and outputs TOTP tokens for manual use.

use akon_core::auth::keyring::retrieve_otp_secret;
use akon_core::auth::totp::generate_totp_default;
use akon_core::config::toml_config::load_config;
use akon_core::error::AkonError;

/// Run the get-password command
///
/// Outputs only the TOTP token to stdout for machine-parsable usage.
/// Errors are sent to stderr. No additional formatting or text.
pub fn run_get_password() -> Result<(), AkonError> {
    // Load configuration to get username
    let config = load_config()?;

    // Retrieve OTP secret from keyring
    let otp_secret_str = retrieve_otp_secret(&config.username)?;

    // Generate TOTP token
    let token = generate_totp_default(&otp_secret_str)?;

    // Output only the token to stdout (machine-parsable)
    println!("{}", token.expose());

    Ok(())
}
