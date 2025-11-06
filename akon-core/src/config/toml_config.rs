//! TOML configuration file I/O
//!
//! Handles loading and saving VPN configuration to/from TOML files
//! in the user's configuration directory.

use crate::config::VpnConfig;
#[cfg(test)]
use crate::config::VpnProtocol;
use crate::error::{AkonError, ConfigError};
use crate::vpn::reconnection::ReconnectionPolicy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Complete TOML configuration structure
///
/// Contains both VPN configuration and reconnection policy settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlConfig {
    /// VPN connection settings
    #[serde(rename = "vpn")]
    pub vpn_config: VpnConfig,

    /// Reconnection policy settings (optional)
    #[serde(rename = "reconnection", default)]
    pub reconnection: Option<ReconnectionPolicy>,
}

impl TomlConfig {
    /// Create a new TOML configuration
    pub fn new(vpn_config: VpnConfig, reconnection: Option<ReconnectionPolicy>) -> Self {
        Self {
            vpn_config,
            reconnection,
        }
    }

    /// Load configuration from a TOML file
    pub fn from_file(path: &Path) -> Result<Self, AkonError> {
        use tracing::{debug, info, warn};

        let contents = std::fs::read_to_string(path).map_err(|e| {
            AkonError::Config(ConfigError::IoError {
                message: format!("Failed to read config file: {}", e),
            })
        })?;

        let config: TomlConfig = toml::from_str(&contents).map_err(|e| {
            AkonError::Config(ConfigError::ValidationError {
                message: format!("Failed to parse config file: {}", e),
            })
        })?;

        // Validate reconnection policy if present
        if let Some(ref policy) = config.reconnection {
            debug!("Validating reconnection policy from config");

            policy.validate().map_err(|e| {
                warn!("Reconnection policy validation failed: {}", e);
                AkonError::Config(ConfigError::ValidationError {
                    message: format!("Invalid reconnection policy: {}", e),
                })
            })?;

            info!(
                "Loaded reconnection policy: max_attempts={}, base_interval={}s, backoff_multiplier={}, max_interval={}s, consecutive_failures={}, health_check_interval={}s, endpoint={}",
                policy.max_attempts,
                policy.base_interval_secs,
                policy.backoff_multiplier,
                policy.max_interval_secs,
                policy.consecutive_failures_threshold,
                policy.health_check_interval_secs,
                policy.health_check_endpoint
            );
        } else {
            debug!("No reconnection policy specified in config, defaults will be used if needed");
        }

        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn to_file(&self, path: &Path) -> Result<(), AkonError> {
        let contents = toml::to_string_pretty(self).map_err(|e| {
            AkonError::Config(ConfigError::ValidationError {
                message: format!("Failed to serialize config: {}", e),
            })
        })?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AkonError::Config(ConfigError::IoError {
                    message: format!("Failed to create config directory: {}", e),
                })
            })?;
        }

        std::fs::write(path, contents).map_err(|e| {
            AkonError::Config(ConfigError::IoError {
                message: format!("Failed to write config file: {}", e),
            })
        })?;

        Ok(())
    }

    /// Get the VPN configuration
    pub fn vpn_config(&self) -> &VpnConfig {
        &self.vpn_config
    }

    /// Get the reconnection policy, or default if not configured
    pub fn reconnection_policy(&self) -> Option<&ReconnectionPolicy> {
        self.reconnection.as_ref()
    }
}

/// Default configuration file name
const CONFIG_FILE_NAME: &str = "config.toml";

/// Get the default configuration directory
///
/// Returns ~/.config/akon on Linux, or AKON_CONFIG_DIR environment variable if set
///
/// With capability-based execution (CAP_NET_ADMIN), this simply uses $HOME since
/// the program runs as the actual user. SUDO_USER fallback is kept for compatibility.
pub fn get_config_dir() -> Result<PathBuf, AkonError> {
    // Allow tests to override config directory via environment variable
    if let Ok(config_dir) = std::env::var("AKON_CONFIG_DIR") {
        return Ok(PathBuf::from(config_dir));
    }

    // Fallback for sudo execution (not needed with capabilities, but kept for compatibility)
    let home = if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        // We're running with sudo, get the actual user's home directory
        std::env::var("SUDO_HOME")
            .or_else(|_: std::env::VarError| {
                // SUDO_HOME not set, construct from /home/username
                Ok::<String, std::env::VarError>(format!("/home/{}", sudo_user))
            })
            .map_err(|_| {
                AkonError::Config(ConfigError::IoError {
                    message: format!("Failed to determine home directory for user: {}", sudo_user),
                })
            })?
    } else {
        // Normal execution, use HOME
        std::env::var("HOME").map_err(|_| {
            AkonError::Config(ConfigError::IoError {
                message: "HOME environment variable not set".to_string(),
            })
        })?
    };

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
    std::fs::create_dir_all(&config_dir).map_err(|e| {
        AkonError::Config(ConfigError::IoError {
            message: format!("Failed to create config directory: {}", e),
        })
    })?;
    Ok(())
}

/// Load VPN configuration from the default TOML file
pub fn load_config() -> Result<VpnConfig, AkonError> {
    let config_path = get_config_path()?;
    load_config_from_path(&config_path)
}

/// Load VPN configuration from a specific TOML file
pub fn load_config_from_path<P: AsRef<Path>>(path: P) -> Result<VpnConfig, AkonError> {
    let contents = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AkonError::Config(ConfigError::LoadFailed {
            path: path.as_ref().to_string_lossy().to_string(),
        }),
        _ => AkonError::Config(ConfigError::IoError {
            message: format!("Failed to read config file: {}", e),
        }),
    })?;

    let config: VpnConfig = toml::from_str(&contents).map_err(|e| {
        AkonError::Config(ConfigError::IoError {
            message: format!("Failed to parse TOML: {}", e),
        })
    })?;

    // Validate the loaded configuration
    config
        .validate()
        .map_err(|e| AkonError::Config(ConfigError::ValidationError { message: e }))?;

    Ok(config)
}

/// Save VPN configuration to the default TOML file
pub fn save_config(config: &VpnConfig) -> Result<(), AkonError> {
    let config_path = get_config_path()?;
    save_config_to_path(config, &config_path)
}

/// Save VPN configuration with reconnection policy to the default TOML file
pub fn save_config_with_reconnection(
    config: &VpnConfig,
    reconnection: Option<&ReconnectionPolicy>,
) -> Result<(), AkonError> {
    let config_path = get_config_path()?;
    save_complete_config_to_path(config, reconnection, &config_path)
}

/// Save VPN configuration to a specific TOML file
pub fn save_config_to_path<P: AsRef<Path>>(config: &VpnConfig, path: P) -> Result<(), AkonError> {
    // Validate configuration before saving
    config
        .validate()
        .map_err(|e| AkonError::Config(ConfigError::ValidationError { message: e }))?;

    // Ensure config directory exists
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AkonError::Config(ConfigError::IoError {
                message: format!("Failed to create config directory: {}", e),
            })
        })?;
    }

    let _e = toml::to_string_pretty(&config)?;

    std::fs::write(&path, _e).map_err(|_e| {
        AkonError::Config(ConfigError::SaveFailed {
            path: path.as_ref().to_string_lossy().to_string(),
        })
    })?;

    Ok(())
}

/// Save complete configuration (VPN + reconnection policy) to a specific TOML file
pub fn save_complete_config_to_path<P: AsRef<Path>>(
    config: &VpnConfig,
    reconnection: Option<&ReconnectionPolicy>,
    path: P,
) -> Result<(), AkonError> {
    use tracing::info;

    // Validate configuration before saving
    config
        .validate()
        .map_err(|e| AkonError::Config(ConfigError::ValidationError { message: e }))?;

    // Validate reconnection policy if present
    if let Some(policy) = reconnection {
        policy.validate().map_err(|e| {
            AkonError::Config(ConfigError::ValidationError {
                message: format!("Invalid reconnection policy: {}", e),
            })
        })?;
    }

    // Ensure config directory exists
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AkonError::Config(ConfigError::IoError {
                message: format!("Failed to create config directory: {}", e),
            })
        })?;
    }

    // Create complete config structure
    let complete_config = TomlConfig::new(config.clone(), reconnection.cloned());

    // Serialize to TOML
    let toml_string = toml::to_string_pretty(&complete_config)?;

    // Write to file
    std::fs::write(&path, toml_string).map_err(|_e| {
        AkonError::Config(ConfigError::SaveFailed {
            path: path.as_ref().to_string_lossy().to_string(),
        })
    })?;

    if reconnection.is_some() {
        info!("Saved VPN configuration with reconnection policy to {:?}", path.as_ref());
    } else {
        info!("Saved VPN configuration to {:?}", path.as_ref());
    }

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
            username: "testuser".to_string(),
            protocol: VpnProtocol::default(),
            timeout: Some(60),
            no_dtls: false,
            lazy_mode: false,
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
            VpnConfig::new("".to_string(), "user".to_string()), // Empty server
            VpnConfig::new("server!".to_string(), "user".to_string()), // Invalid chars
            VpnConfig::new("server.com".to_string(), "".to_string()), // Empty username
        ];

        for config in invalid_configs {
            assert!(config.validate().is_err());
        }
    }
}
