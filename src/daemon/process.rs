//! Daemon process management
//!
//! Handles spawning daemon processes, PID file management, and daemon lifecycle.

use akon_core::error::{AkonError, VpnError};
use tracing::{debug, info, warn};

/// Cleanup orphaned OpenConnect processes (T049)
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
    use nix::errno::Errno;
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use std::process::{Command, Stdio};
    use tracing::{debug, warn};

    enum SignalResult {
        Delivered,
        AlreadyExited,
        NotPermitted,
        Failed,
    }

    fn attempt_privileged_kill(pid: i32, signal: Signal) -> bool {
        let signal_arg = match signal {
            Signal::SIGTERM => "-TERM",
            Signal::SIGKILL => "-KILL",
            _ => return false,
        };

        match Command::new("sudo")
            .arg("-n")
            .arg("kill")
            .arg(signal_arg)
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {
                debug!("Elevated kill succeeded for process {} with {:?}", pid, signal);
                true
            }
            Ok(status) => {
                warn!(
                    "sudo kill exited with status {:?} when sending {:?} to process {}",
                    status.code(),
                    signal,
                    pid
                );
                false
            }
            Err(e) => {
                warn!(
                    "Failed to invoke sudo when sending {:?} to process {}: {}",
                    signal,
                    pid,
                    e
                );
                false
            }
        }
    }

    fn is_process_running(pid: i32) -> bool {
        Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn send_signal(pid: i32, signal: Signal) -> SignalResult {
        let pid_obj = Pid::from_raw(pid);

        match kill(pid_obj, signal) {
            Ok(_) => SignalResult::Delivered,
            Err(Errno::ESRCH) => SignalResult::AlreadyExited,
            Err(Errno::EPERM) => {
                if attempt_privileged_kill(pid, signal) {
                    SignalResult::Delivered
                } else if !is_process_running(pid) {
                    SignalResult::AlreadyExited
                } else {
                    SignalResult::NotPermitted
                }
            }
            Err(err) => {
                warn!("Failed to send {:?} to process {}: {}", signal, pid, err);
                SignalResult::Failed
            }
        }
    }

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
        debug!("Sending SIGTERM to process {}", pid);

        match send_signal(pid, Signal::SIGTERM) {
            SignalResult::Delivered => {
                // Wait for graceful shutdown
                std::thread::sleep(std::time::Duration::from_secs(5));

                if is_process_running(pid) {
                    warn!(
                        "Process {} did not respond to SIGTERM, sending SIGKILL",
                        pid
                    );

                    match send_signal(pid, Signal::SIGKILL) {
                        SignalResult::Delivered => {
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            if is_process_running(pid) {
                                warn!(
                                    "Process {} still running after SIGKILL; manual intervention required",
                                    pid
                                );
                            } else {
                                info!(
                                    "Successfully terminated process {} with SIGKILL",
                                    pid
                                );
                                terminated_count += 1;
                            }
                        }
                        SignalResult::AlreadyExited => {
                            debug!(
                                "Process {} exited while escalating to SIGKILL",
                                pid
                            );
                            terminated_count += 1;
                        }
                        SignalResult::NotPermitted => {
                            warn!(
                                "Insufficient privileges to forcefully terminate process {}. Run akon with sudo or configure passwordless sudo for kill/openconnect.",
                                pid
                            );
                        }
                        SignalResult::Failed => {
                            // Error already logged inside send_signal
                        }
                    }
                } else {
                    info!("Process {} terminated gracefully", pid);
                    terminated_count += 1;
                }
            }
            SignalResult::AlreadyExited => {
                debug!("Process {} already terminated", pid);
                terminated_count += 1;
            }
            SignalResult::NotPermitted => {
                warn!(
                    "Insufficient privileges to terminate process {}. Run akon with sudo or configure passwordless sudo for kill/openconnect.",
                    pid
                );
            }
            SignalResult::Failed => {
                // Error already logged inside send_signal
            }
        }
    }

    info!(
        "Cleanup complete: terminated {}/{} processes",
        terminated_count, total_pids
    );
    Ok(terminated_count)
}
