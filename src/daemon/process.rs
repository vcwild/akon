//! Daemon process management
//!
//! Handles spawning daemon processes, PID file management, and daemon lifecycle.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use daemonize::Daemonize;
use tracing::info;

use akon_core::error::{AkonError, VpnError};

/// Represents a daemon process
pub struct DaemonProcess {
    pid_file: PathBuf,
}

impl DaemonProcess {
    /// Create a new daemon process manager
    pub fn new(pid_file: PathBuf) -> Self {
        Self { pid_file }
    }

    /// Check if a daemon is already running
    pub fn is_running(&self) -> Result<bool, AkonError> {
        if !self.pid_file.exists() {
            return Ok(false);
        }

        let pid_content = fs::read_to_string(&self.pid_file)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read PID file: {}", e),
            }))?;

        let pid: i32 = pid_content.trim().parse()
            .map_err(|_| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid PID in PID file".to_string(),
            }))?;

        // Check if process is running
        match nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid))) {
            Ok(_) => Ok(true),
            Err(nix::errno::Errno::ESRCH) => {
                // Process doesn't exist, clean up PID file
                let _ = fs::remove_file(&self.pid_file);
                Ok(false)
            }
            Err(e) => Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to check process status: {}", e),
            })),
        }
    }

    /// Daemonize the current process
    pub fn daemonize(&self) -> Result<(), AkonError> {
        // Ensure PID file directory exists
        if let Some(parent) = self.pid_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to create PID file directory: {}", e),
                }))?;
        }

        let daemonize = Daemonize::new()
            .pid_file(&self.pid_file)
            .chown_pid_file(true)
            .working_directory(std::env::current_dir()
                .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to get current directory: {}", e),
                }))?
            )
            .umask(0o027); // Restrictive permissions

        daemonize.start()
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to daemonize process: {}", e),
            }))?;

        info!("Successfully daemonized process, PID: {}", process::id());
        Ok(())
    }

    /// Get the PID of the running daemon
    pub fn get_pid(&self) -> Result<i32, AkonError> {
        if !self.pid_file.exists() {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "No PID file found".to_string(),
            }));
        }

        let pid_content = fs::read_to_string(&self.pid_file)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read PID file: {}", e),
            }))?;

        pid_content.trim().parse()
            .map_err(|_| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid PID in PID file".to_string(),
            }))
    }

    /// Stop the daemon process
    pub fn stop(&self) -> Result<(), AkonError> {
        let pid = self.get_pid()?;

        // Send SIGTERM to the process
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        )
        .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to send SIGTERM to daemon: {}", e),
        }))?;

        // Wait a bit for graceful shutdown
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Check if it's still running
        if self.is_running()? {
            // Send SIGKILL if still running
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            )
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to send SIGKILL to daemon: {}", e),
            }))?;
        }

        // Clean up PID file
        let _ = fs::remove_file(&self.pid_file);

        info!("Stopped daemon process {}", pid);
        Ok(())
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        // Clean up PID file if it exists
        let _ = fs::remove_file(&self.pid_file);
    }
}

/// Get the default PID file path
pub fn get_default_pid_file() -> PathBuf {
    // Use XDG_RUNTIME_DIR if available, otherwise /tmp
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        Path::new(&runtime_dir).join("akon.pid")
    } else {
        Path::new("/tmp").join(format!("akon-{}.pid", nix::unistd::getuid()))
    }
}
