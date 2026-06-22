//! Background mode for `akon vpn on`.
//!
//! When `--foreground` is NOT given (the default), `vpn on` returns the terminal
//! to the user as soon as the VPN reaches `Connected`. The VPN supervisor
//! continues running as a background process.
//!
//! # How it works
//!
//! Because forking inside a multi-threaded tokio runtime is unsafe, we use
//! **re-exec** instead: the parent spawns itself (`akon vpn on --foreground`)
//! as a background child with stdin/stdout/stderr redirected to the VPN log
//! file. Communication from child → parent (the connect result: IP, device, or
//! error) is via a small **ready file** that the child writes once `Connected`
//! (or on failure) before the parent has exited.
//!
//! Flow:
//! 1. Parent generates PIN+OTP and passes it to the child via `AKON_VPN_PASSWORD`.
//! 2. Parent spawns `akon vpn on --foreground` with stdio → log file.
//! 3. Child connects; on `Connected` it writes `<ip>\t<device>` to a temp ready
//!    file, then continues running the VPN in the background.
//! 4. Parent polls the ready file (bounded: 60 s). On success, prints the
//!    connected summary and returns (exit 0). On timeout or child exit, prints
//!    the error (exit 1).

#[cfg(target_os = "linux")]
mod imp {
    use akon_core::auth::password::generate_password;
    use akon_core::config::VpnConfig;
    use akon_core::error::{AkonError, VpnError};
    use akon_core::vpn::reconnection::ReconnectionPolicy;
    use colored::Colorize;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// Path to the VPN background-process log file.
    pub fn vpn_log_path() -> PathBuf {
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/tmp"))
                    .join(".local/share")
            });
        base.join("akon/vpn.log")
    }

    /// Launch `akon vpn on --foreground` in the background, wait for it to
    /// signal `Connected`, print the summary, and return — giving the terminal
    /// back to the user. The supervisor continues running in the background.
    pub async fn run_vpn_on_background(
        config: &VpnConfig,
        state_path: &Path,
        _reconnection: Option<ReconnectionPolicy>,
    ) -> Result<(), AkonError> {
        // Generate PIN+OTP now (as the current user, keyring accessible).
        let password = match std::env::var("AKON_VPN_PASSWORD") {
            Ok(p) if !p.trim().is_empty() => p,
            _ => generate_password(&config.username)
                .map_err(|e| {
                    AkonError::Vpn(VpnError::ConnectionFailed {
                        reason: format!("Failed to generate password: {e}"),
                    })
                })?
                .expose()
                .to_string(),
        };

        // A temp file the child writes to once Connected (or on failure).
        // Format on success: "OK\t<ip>\t<device>"
        // Format on failure: "FAIL\t<error message>"
        let ready_file =
            std::env::temp_dir().join(format!("akon_ready_{}.txt", std::process::id()));
        let _ = fs::remove_file(&ready_file); // clean any leftover

        // Ensure the log directory exists.
        let log_path = vpn_log_path();
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Open (or create/append) the log file for the child's stdio.
        let log_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to open log file {}: {e}", log_path.display()),
                })
            })?;
        let log_file2 = log_file.try_clone().map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to clone log file handle: {e}"),
            })
        })?;

        // The current executable.
        let exe = std::env::current_exe().map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to locate current executable: {e}"),
            })
        })?;

        // Build the child argv. Pass reconnection serialised if present.
        let mut child_cmd = std::process::Command::new(&exe);
        child_cmd
            .args(["vpn", "on", "--foreground"])
            .env("AKON_VPN_PASSWORD", &password)
            .env("AKON_STATE_FILE", state_path.as_os_str())
            .env("AKON_BACKGROUND_READY_FILE", ready_file.as_os_str())
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file2));

        // Propagate debug flag.
        if std::env::var("AKON_F5_DEBUG").as_deref() == Ok("1") {
            child_cmd.env("AKON_F5_DEBUG", "1");
        }

        println!(
            "{} {} {}",
            ">>".bright_cyan(),
            "Connecting to VPN server (native F5):"
                .bright_white()
                .bold(),
            config.server.bright_yellow()
        );

        let _child = child_cmd.spawn().map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to spawn background VPN process: {e}"),
            })
        })?;
        // Note: we intentionally do NOT call child.wait() — we want it to
        // continue running after this function returns.

        // Poll for the ready file (bounded: 60 s). The child writes it once
        // Connected or on failure.
        let timeout = Duration::from_secs(60);
        let start = Instant::now();
        let ready = loop {
            if start.elapsed() >= timeout {
                break None;
            }
            if let Ok(content) = fs::read_to_string(&ready_file) {
                let _ = fs::remove_file(&ready_file);
                break Some(content);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let _ = fs::remove_file(&ready_file);

        match ready {
            Some(content) if content.starts_with("OK\t") => {
                let parts: Vec<&str> = content.trim().splitn(3, '\t').collect();
                let ip = parts.get(1).copied().unwrap_or("unknown");
                let device = parts.get(2).copied().unwrap_or("unknown");
                println!(
                    "{} {}",
                    "[OK]".bright_green().bold(),
                    "VPN connection established".bright_green().bold()
                );
                println!(
                    "   {} {}",
                    "IP address:".bright_white(),
                    ip.bright_cyan().bold()
                );
                println!("   {} {}", "Device:".bright_white(), device.bright_cyan());
                println!(
                    "   {} {}",
                    "Logs:".dimmed(),
                    log_path.display().to_string().dimmed()
                );
                println!(
                    "\n   {} {} to disconnect",
                    "Run".dimmed(),
                    "akon vpn off".bright_cyan()
                );
                Ok(())
            }
            Some(content) if content.starts_with("FAIL\t") => {
                let msg = content
                    .trim()
                    .strip_prefix("FAIL\t")
                    .unwrap_or("connection failed");
                Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: msg.to_string(),
                }))
            }
            _ => Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!(
                    "VPN did not signal ready within 60s; check logs at {}",
                    log_path.display()
                ),
            })),
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::run_vpn_on_background;

/// Write the connect result to the ready file so the parent process (which
/// spawned us as a background child) can read it and print the summary.
/// Called from `native_connect_once` once `Connected` (or `Failed`) is received.
pub fn signal_ready(result: BackgroundReady) {
    let Some(path) = std::env::var_os("AKON_BACKGROUND_READY_FILE") else {
        return; // not running as a background child
    };
    let content = match result {
        BackgroundReady::Connected { ip, device } => format!("OK\t{ip}\t{device}"),
        BackgroundReady::Failed { message } => format!("FAIL\t{message}"),
    };
    let _ = std::fs::write(path, content);
}

/// The result written to the ready file.
pub enum BackgroundReady {
    Connected { ip: String, device: String },
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_ready_writes_ok_format() {
        let tmp = std::env::temp_dir().join("akon_bg_test_ok.txt");
        let _ = std::fs::remove_file(&tmp);
        // Simulate running as background child.
        std::env::set_var("AKON_BACKGROUND_READY_FILE", tmp.as_os_str());
        signal_ready(BackgroundReady::Connected {
            ip: "10.20.30.40".into(),
            device: "tun0".into(),
        });
        std::env::remove_var("AKON_BACKGROUND_READY_FILE");
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "OK\t10.20.30.40\ttun0");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn signal_ready_writes_fail_format() {
        let tmp = std::env::temp_dir().join("akon_bg_test_fail.txt");
        let _ = std::fs::remove_file(&tmp);
        std::env::set_var("AKON_BACKGROUND_READY_FILE", tmp.as_os_str());
        signal_ready(BackgroundReady::Failed {
            message: "bad credentials".into(),
        });
        std::env::remove_var("AKON_BACKGROUND_READY_FILE");
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "FAIL\tbad credentials");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn signal_ready_is_no_op_when_env_unset() {
        std::env::remove_var("AKON_BACKGROUND_READY_FILE");
        // Should not panic or write any file.
        signal_ready(BackgroundReady::Connected {
            ip: "1.2.3.4".into(),
            device: "tun0".into(),
        });
    }
}
