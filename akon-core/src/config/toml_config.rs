//! TOML configuration file I/O
//!
//! Handles loading and saving VPN configuration to/from TOML files
//! in the user's configuration directory.

use std::path::{Path, PathBuf};
use crate::config::VpnConfig;
use crate::error::{AkonError, ConfigError};

/// Default configuration file name
const CONFIG_FILE_NAME: &str = "config.toml";

/// Get the default configuration directory
///
/// Returns ~/.config/akon on Linux
pub fn get_config_dir() -> Result<PathBuf, AkonError> {
    let home = std::env::var("HOME")
        .map_err(|_| AkonError::Config(ConfigError::IoError { message: "HOME environment variable not set".to_string() }))?;

    let config_dir = PathBuf::from(home).join(".config").join("akon");
    Ok(config_dir)
}

/// Get the default configuration file path
pub fn get_config_path() -> Result<PathBuf, AkonError> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join(CONFIG_FILE_NAME))
}

/// Ensure the configuration directory exists
pub fn ensure_config_dir() -> Result<(), AkonError> {
    let config_dir = get_config_dir()?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| AkonError::Config(ConfigError::IoError { message: format!("Failed to create config directory: {}", e) }))?;
    Ok(())
}

/// Load VPN configuration from the default TOML file
pub fn load_config() -> Result<VpnConfig, AkonError> {
    let config_path = get_config_path()?;
    load_config_from_path(&config_path)
}

/// Load VPN configuration from a specific TOML file
pub fn load_config_from_path<P: AsRef<Path>>(path: P) -> Result<VpnConfig, AkonError> {
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AkonError::Config(ConfigError::LoadFailed { path: path.as_ref().to_string_lossy().to_string() }),
            _ => AkonError::Config(ConfigError::IoError { message: format!("Failed to read config file: {}", e) }),
        })?;

    let config: VpnConfig = toml::from_str(&contents)
        .map_err(|e| AkonError::Config(ConfigError::IoError { message: format!("Failed to parse TOML: {}", e) }))?;

    // Validate the loaded configuration
    config.validate()
        .map_err(|e| AkonError::Config(ConfigError::ValidationError { message: e }))?;

    Ok(config)
}

/// Save VPN configuration to the default TOML file
pub fn save_config(config: &VpnConfig) -> Result<(), AkonError> {
    let config_path = get_config_path()?;
    save_config_to_path(config, &config_path)
}

/// Save VPN configuration to a specific TOML file
pub fn save_config_to_path<P: AsRef<Path>>(config: &VpnConfig, path: P) -> Result<(), AkonError> {
    // Validate configuration before saving
    config.validate()
        .map_err(|e| AkonError::Config(ConfigError::ValidationError { message: e }))?;

    // Ensure config directory exists
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AkonError::Config(ConfigError::IoError { message: format!("Failed to create config directory: {}", e) }))?;
    }

    let _e = toml::to_string_pretty(&config)?;

    std::fs::write(&path, _e)
        .map_err(|_e| AkonError::Config(ConfigError::SaveFailed { path: path.as_ref().to_string_lossy().to_string() }))?;

    Ok(())
}

/// Check if a configuration file exists
pub fn config_exists() -> Result<bool, AkonError> {
    let config_path = get_config_path()?;
    Ok(config_path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");

        let original_config = VpnConfig {
            server: "vpn.example.com".to_string(),
            port: 4443,
            username: "testuser".to_string(),
            realm: Some("realm1".to_string()),
            timeout: Some(60),
        };

        // Save config
        save_config_to_path(&original_config, &config_path).unwrap();

        // Load config
        let loaded_config = load_config_from_path(&config_path).unwrap();

        assert_eq!(original_config, loaded_config);
    }

    #[test]
    fn test_invalid_config_validation() {
        let invalid_configs = vec![
            VpnConfig::new("".to_string(), 443, "user".to_string()), // Empty server
            VpnConfig::new("server!".to_string(), 443, "user".to_string()), // Invalid chars
            VpnConfig::new("server.com".to_string(), 0, "user".to_string()), // Invalid port
            VpnConfig::new("server.com".to_string(), 443, "".to_string()), // Empty username
        ];

        for config in invalid_configs {
            assert!(config.validate().is_err());
        }
    }
}
