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
#[allow(dead_code)]
pub struct DaemonProcess {
    pid_file: PathBuf,
}

#[allow(dead_code)]
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

        let pid_content = fs::read_to_string(&self.pid_file).map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read PID file: {}", e),
            })
        })?;

        let pid: i32 = pid_content.trim().parse().map_err(|_| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid PID in PID file".to_string(),
            })
        })?;

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
            fs::create_dir_all(parent).map_err(|e| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to create PID file directory: {}", e),
                })
            })?;
        }

        let daemonize = Daemonize::new()
            .pid_file(&self.pid_file)
            .chown_pid_file(true)
            .working_directory(std::env::current_dir().map_err(|e| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to get current directory: {}", e),
                })
            })?)
            .umask(0o027); // Restrictive permissions

        daemonize.start().map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to daemonize process: {}", e),
            })
        })?;

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

        let pid_content = fs::read_to_string(&self.pid_file).map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read PID file: {}", e),
            })
        })?;

        pid_content.trim().parse().map_err(|_| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid PID in PID file".to_string(),
            })
        })
    }

    /// Stop the daemon process
    pub fn stop(&self) -> Result<(), AkonError> {
        let pid = self.get_pid()?;

        // Send SIGTERM to the process
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        )
        .map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to send SIGTERM to daemon: {}", e),
            })
        })?;

        // Wait a bit for graceful shutdown
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Check if it's still running
        if self.is_running()? {
            // Send SIGKILL if still running
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            )
            .map_err(|e| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to send SIGKILL to daemon: {}", e),
                })
            })?;
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
#[allow(dead_code)]
pub fn get_default_pid_file() -> PathBuf {
    // Use XDG_RUNTIME_DIR if available, otherwise /tmp
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        Path::new(&runtime_dir).join("akon.pid")
    } else {
        Path::new("/tmp").join(format!("akon-{}.pid", nix::unistd::getuid()))
    }
}

/// Cleanup orphaned OpenConnect processes (T049)
///
/// Finds all OpenConnect processes and terminates them gracefully (SIGTERM),
/// then forcefully (SIGKILL) if they don't respond within 5 seconds.
///
/// Returns the number of processes successfully terminated.
///
/// # Errors
///
/// Returns an error if:
/// - Unable to list running processes
/// - All termination attempts fail (but logs individual failures)
///
/// # Example
///
/// ```no_run
/// use akon::daemon::process::cleanup_orphaned_processes;
///
/// match cleanup_orphaned_processes() {
///     Ok(count) => println!("Terminated {} orphaned processes", count),
///     Err(e) => eprintln!("Cleanup failed: {}", e),
/// }
/// ```
pub fn cleanup_orphaned_processes() -> Result<usize, AkonError> {
    use std::process::Command;
    use tracing::{debug, warn};

    // Find all openconnect processes
    let output = Command::new("pgrep")
        .arg("-x") // Exact match
        .arg("openconnect")
        .output()
        .map_err(|e| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to search for openconnect processes: {}", e),
            })
        })?;

    if !output.status.success() {
        // No processes found (pgrep returns non-zero when no matches)
        debug!("No openconnect processes found");
        return Ok(0);
    }

    let pids_str = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<i32> = pids_str
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();

    if pids.is_empty() {
        debug!("No openconnect processes to cleanup");
        return Ok(0);
    }

    let total_pids = pids.len();
    info!(
        "Found {} openconnect process(es) to cleanup: {:?}",
        total_pids, pids
    );

    let mut terminated_count = 0;

    for pid in pids {
        let pid_obj = nix::unistd::Pid::from_raw(pid);

        // Step 1: Send SIGTERM for graceful shutdown
        debug!("Sending SIGTERM to process {}", pid);
        match nix::sys::signal::kill(pid_obj, nix::sys::signal::Signal::SIGTERM) {
            Ok(_) => {
                debug!("SIGTERM sent to process {}", pid);
            }
            Err(nix::errno::Errno::ESRCH) => {
                // Process already terminated
                debug!("Process {} already terminated", pid);
                terminated_count += 1;
                continue;
            }
            Err(nix::errno::Errno::EPERM) => {
                warn!(
                    "Permission denied to terminate process {} (owned by different user)",
                    pid
                );
                continue;
            }
            Err(e) => {
                warn!("Failed to send SIGTERM to process {}: {}", pid, e);
                continue;
            }
        }

        // Step 2: Wait 5 seconds for graceful shutdown
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Step 3: Check if process still exists
        match nix::sys::signal::kill(pid_obj, None) {
            Ok(_) => {
                // Process still running, need SIGKILL
                warn!(
                    "Process {} did not respond to SIGTERM, sending SIGKILL",
                    pid
                );
                match nix::sys::signal::kill(pid_obj, nix::sys::signal::Signal::SIGKILL) {
                    Ok(_) => {
                        info!("Successfully terminated process {} with SIGKILL", pid);
                        terminated_count += 1;
                    }
                    Err(nix::errno::Errno::ESRCH) => {
                        // Process terminated between check and SIGKILL
                        debug!("Process {} terminated before SIGKILL", pid);
                        terminated_count += 1;
                    }
                    Err(e) => {
                        warn!("Failed to send SIGKILL to process {}: {}", pid, e);
                    }
                }
            }
            Err(nix::errno::Errno::ESRCH) => {
                // Process terminated gracefully
                info!("Process {} terminated gracefully", pid);
                terminated_count += 1;
            }
            Err(e) => {
                warn!("Error checking process {} status: {}", pid, e);
            }
        }
    }

    info!(
        "Cleanup complete: terminated {}/{} processes",
        terminated_count, total_pids
    );
    Ok(terminated_count)
}
