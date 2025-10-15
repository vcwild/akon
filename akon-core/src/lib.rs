//! Core library for akon VPN CLI tool
//!
//! This crate provides the core functionality for secure credential management
//! and VPN connection handling.

pub mod error;
pub mod types;

pub mod auth;
pub mod config;
pub mod vpn;

/// Initialize logging infrastructure
///
/// Sets up tracing with systemd journal logging for production use.
/// In development, logs to stderr with appropriate formatting.
pub fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    // Try to use systemd journal logging if available
    #[cfg(target_os = "linux")]
    {
        if std::env::var("JOURNAL_STREAM").is_ok() {
            // We're running under systemd, use journal logging
            let journal_layer = tracing_journald::layer()?;
            tracing_subscriber::registry()
                .with(journal_layer)
                .with(tracing_subscriber::filter::LevelFilter::INFO)
                .init();
            return Ok(());
        }
    }

    // Fallback to stderr logging with pretty formatting
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().pretty())
        .with(tracing_subscriber::filter::LevelFilter::INFO)
        .init();

    Ok(())
}
