//! OpenConnect process management and cleanup
//!
//! This module provides functions to find, terminate, and cleanup
//! OpenConnect VPN processes.

use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

/// Error types for process operations
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Failed to find process: {0}")]
    ProcessNotFound(String),

    #[error("Failed to terminate process: {0}")]
    TerminationFailed(String),

    #[error("Process did not respond to signals")]
    UnresponsiveProcess,
}

/// Find OpenConnect processes by PID
///
/// # Arguments
///
/// * `pid` - Process ID to check
///
/// # Returns
///
/// True if the process exists and is an openconnect process
pub fn is_process_alive(pid: u32) -> bool {
    // Check if process exists using ps
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                let comm = String::from_utf8_lossy(&out.stdout);
                comm.trim().contains("openconnect")
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Terminate an OpenConnect process gracefully
///
/// Sends SIGTERM first, waits up to 5 seconds, then sends SIGKILL if still alive.
///
/// # Arguments
///
/// * `pid` - Process ID to terminate
///
/// # Returns
///
/// Result indicating success or failure
pub async fn terminate_process(pid: u32) -> Result<(), ProcessError> {
    // Check if process exists
    if !is_process_alive(pid) {
        return Ok(()); // Already terminated
    }

    // Send SIGTERM (graceful termination)
    let sigterm_result = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();

    if let Err(e) = sigterm_result {
        return Err(ProcessError::TerminationFailed(format!(
            "Failed to send SIGTERM: {}",
            e
        )));
    }

    // Wait up to 5 seconds for graceful termination
    for _ in 0..10 {
        sleep(Duration::from_millis(500)).await;
        if !is_process_alive(pid) {
            return Ok(());
        }
    }

    // Process still alive, send SIGKILL (forceful termination)
    let sigkill_result = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output();

    if let Err(e) = sigkill_result {
        return Err(ProcessError::TerminationFailed(format!(
            "Failed to send SIGKILL: {}",
            e
        )));
    }

    // Wait briefly for SIGKILL to take effect
    sleep(Duration::from_millis(500)).await;

    if is_process_alive(pid) {
        Err(ProcessError::UnresponsiveProcess)
    } else {
        Ok(())
    }
}

/// Find and terminate all OpenConnect processes
///
/// Uses pgrep to find all openconnect processes and terminates them.
///
/// # Returns
///
/// Vector of PIDs that were terminated
pub async fn cleanup_all_openconnect_processes() -> Result<Vec<u32>, ProcessError> {
    // Find all openconnect processes
    let output = Command::new("pgrep")
        .arg("openconnect")
        .output()
        .map_err(|e| ProcessError::ProcessNotFound(format!("pgrep failed: {}", e)))?;

    if !output.status.success() {
        // No processes found
        return Ok(vec![]);
    }

    let pids_str = String::from_utf8_lossy(&output.stdout);
    let mut terminated_pids = vec![];

    for line in pids_str.lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            if terminate_process(pid).await.is_ok() {
                terminated_pids.push(pid);
            }
        }
    }

    Ok(terminated_pids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_process_alive_with_nonexistent_pid() {
        // PID 99999999 should not exist
        assert!(!is_process_alive(99999999));
    }

    #[test]
    fn test_is_process_alive_with_pid_1() {
        // PID 1 (init/systemd) should always exist but not be openconnect
        let alive = is_process_alive(1);
        // This will be false because PID 1 is not openconnect
        assert!(!alive);
    }

    #[tokio::test]
    async fn test_terminate_nonexistent_process() {
        // Should succeed (process already gone)
        let result = terminate_process(99999999).await;
        assert!(result.is_ok());
    }
}
