# Contract: VPN Off Command (Graceful Termination)

**Feature**: 002-refactor-openconnect-to
**Module**: `src/cli/vpn.rs::off_command()`
**Purpose**: Gracefully terminate OpenConnect VPN connection with SIGTERM→SIGKILL fallback

## Interface

### Function Signature

```rust
pub async fn off_command() -> Result<(), AkonError>
```

### Parameters

None (operates on current VPN connection state)

### Returns

- `Ok(())` - VPN disconnected successfully
- `Err(AkonError)` - Disconnection failed or no active connection

### Error Cases

```rust
pub enum AkonError {
    VpnError(VpnError),
    StateError(String),  // No active connection found
}

pub enum VpnError {
    TerminationError(String),  // Failed to terminate process
    ProcessNotFound(String),   // OpenConnect process not running
}
```

## Behavior

### Pre-conditions

1. Active VPN connection exists (verified via state file)
2. OpenConnect process running (PID tracked in state)
3. User has appropriate permissions to send signals to process

### Post-conditions (Success)

1. OpenConnect process terminated
2. TUN device removed
3. State file deleted
4. User sees disconnection confirmation
5. Exit code: 0

### Post-conditions (Failure - No Connection)

1. User informed no connection exists
2. State file checked and cleaned if stale
3. Exit code: 1

### Post-conditions (Failure - Termination Error)

1. Force-kill attempted as fallback
2. State file cleaned even if process kill fails
3. User sees error with diagnostic info
4. Exit code: 1

## Implementation Contract

### Step 1: Load Connection State

```rust
use std::fs;
use serde_json;

fn load_connection_state() -> Result<ConnectionState, AkonError> {
    let state_path = "/tmp/akon_vpn_state.json";

    if !std::path::Path::new(state_path).exists() {
        return Err(AkonError::StateError(
            "No active VPN connection found".to_string()
        ));
    }

    let state_json = fs::read_to_string(state_path)
        .map_err(|e| AkonError::StateError(format!("Failed to read state: {}", e)))?;

    let state: ConnectionState = serde_json::from_str(&state_json)
        .map_err(|e| AkonError::StateError(format!("Invalid state file: {}", e)))?;

    Ok(state)
}

#[derive(Debug, serde::Deserialize)]
struct ConnectionState {
    ip: std::net::IpAddr,
    device: String,
    connected_at: chrono::DateTime<chrono::Utc>,
    pid: Option<u32>,  // OpenConnect process PID
}
```

### Step 2: Verify Process Still Running

```rust
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

fn verify_process_running(pid: u32) -> Result<(), AkonError> {
    // Signal 0 is null signal (doesn't send signal, just checks if process exists)
    match kill(Pid::from_raw(pid as i32), Signal::SIGNULL) {
        Ok(_) => Ok(()),
        Err(nix::errno::Errno::ESRCH) => {
            // Process not found
            Err(AkonError::VpnError(VpnError::ProcessNotFound(
                format!("OpenConnect process {} not running", pid)
            )))
        }
        Err(e) => Err(AkonError::VpnError(VpnError::TerminationError(
            format!("Failed to check process status: {}", e)
        )))
    }
}
```

### Step 3: Graceful Termination (SIGTERM)

```rust
use tokio::time::{timeout, Duration};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

async fn disconnect_gracefully(pid: u32) -> Result<(), AkonError> {
    println!("Disconnecting VPN...");

    let pid_obj = Pid::from_raw(pid as i32);

    // Send SIGTERM
    kill(pid_obj, Signal::SIGTERM)
        .map_err(|e| AkonError::VpnError(VpnError::TerminationError(
            format!("Failed to send SIGTERM: {}", e)
        )))?;

    tracing::info!(pid = %pid, signal = "SIGTERM", "Sent termination signal");

    // Wait up to 5 seconds for graceful exit
    let result = timeout(Duration::from_secs(5), wait_for_process_exit(pid)).await;

    match result {
        Ok(Ok(())) => {
            println!("✓ VPN disconnected gracefully");
            tracing::info!(pid = %pid, "Process exited gracefully");
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout - need force kill
            tracing::warn!(pid = %pid, "Graceful shutdown timeout, force killing");
            force_kill_process(pid).await
        }
    }
}

async fn wait_for_process_exit(pid: u32) -> Result<(), AkonError> {
    let pid_obj = Pid::from_raw(pid as i32);

    // Poll every 100ms to check if process exited
    for _ in 0..50 {  // 50 * 100ms = 5 seconds max
        tokio::time::sleep(Duration::from_millis(100)).await;

        match kill(pid_obj, Signal::SIGNULL) {
            Err(nix::errno::Errno::ESRCH) => {
                // Process no longer exists
                return Ok(());
            }
            Ok(_) => {
                // Process still running, continue waiting
                continue;
            }
            Err(e) => {
                return Err(AkonError::VpnError(VpnError::TerminationError(
                    format!("Error checking process: {}", e)
                )));
            }
        }
    }

    Err(AkonError::VpnError(VpnError::TerminationError(
        "Process did not exit within timeout".to_string()
    )))
}
```

### Step 4: Force Kill (SIGKILL) Fallback

```rust
async fn force_kill_process(pid: u32) -> Result<(), AkonError> {
    println!("  Force-killing VPN process...");

    let pid_obj = Pid::from_raw(pid as i32);

    // Send SIGKILL (non-catchable)
    kill(pid_obj, Signal::SIGKILL)
        .map_err(|e| AkonError::VpnError(VpnError::TerminationError(
            format!("Failed to send SIGKILL: {}", e)
        )))?;

    tracing::warn!(pid = %pid, signal = "SIGKILL", "Force-killed process");

    // Wait briefly to confirm termination
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify process is gone
    match kill(pid_obj, Signal::SIGNULL) {
        Err(nix::errno::Errno::ESRCH) => {
            println!("✓ VPN disconnected (forced)");
            Ok(())
        }
        Ok(_) => {
            // Process STILL running after SIGKILL (very rare, usually unkillable kernel process)
            Err(AkonError::VpnError(VpnError::TerminationError(
                format!("Process {} could not be killed", pid)
            )))
        }
        Err(e) => Err(AkonError::VpnError(VpnError::TerminationError(
            format!("Error verifying process termination: {}", e)
        )))
    }
}
```

### Step 5: Clean Up State File

```rust
fn cleanup_state_file() -> Result<(), AkonError> {
    let state_path = "/tmp/akon_vpn_state.json";

    if std::path::Path::new(state_path).exists() {
        fs::remove_file(state_path)
            .map_err(|e| AkonError::StateError(format!("Failed to remove state file: {}", e)))?;

        tracing::debug!("State file cleaned up");
    }

    Ok(())
}
```

### Complete off_command Implementation

```rust
pub async fn off_command() -> Result<(), AkonError> {
    // Step 1: Load state
    let state = match load_connection_state() {
        Ok(s) => s,
        Err(AkonError::StateError(msg)) => {
            println!("ℹ {}", msg);

            // Clean up stale state file if it exists
            let _ = cleanup_state_file();

            return Err(AkonError::StateError(msg));
        }
        Err(e) => return Err(e),
    };

    // Step 2: Verify process (if PID tracked)
    if let Some(pid) = state.pid {
        if let Err(e) = verify_process_running(pid) {
            tracing::warn!(
                pid = %pid,
                error = %e,
                "Process not running, cleaning up state"
            );

            println!("ℹ OpenConnect process not running (already disconnected?)");
            cleanup_state_file()?;
            return Ok(());
        }

        // Step 3: Graceful termination
        let disconnect_result = disconnect_gracefully(pid).await;

        // Step 4: Clean up state (even if disconnect failed)
        cleanup_state_file()?;

        disconnect_result
    } else {
        // No PID tracked - manual cleanup
        println!("⚠ No process ID tracked, cleaning up state");
        cleanup_state_file()?;

        Err(AkonError::StateError(
            "No OpenConnect process ID found in state".to_string()
        ))
    }
}
```

## Performance Requirements

- **Graceful Shutdown Time**: <5 seconds (SIGTERM timeout)
- **Force Kill Time**: <500ms (SIGKILL + verification)
- **Total Max Time**: <6 seconds (worst case)

## Security Requirements

1. **Process Ownership Verification**:
   - Only kill processes owned by current user (or root)
   - Prevent killing arbitrary system processes

2. **Signal Safety**:
   - Use null signal (0) to check process existence
   - SIGTERM before SIGKILL (allows cleanup)

3. **State File Access**:
   - Verify state file is not world-writable
   - Validate PID before sending signals

## Observability (FR-022)

```rust
use tracing::{info, warn, error};

// Disconnection started
info!(
    pid = %pid,
    ip = %state.ip,
    device = %state.device,
    duration_seconds = %(chrono::Utc::now() - state.connected_at).num_seconds(),
    "VPN disconnection initiated"
);

// Graceful shutdown success
info!(
    pid = %pid,
    method = "SIGTERM",
    state = "Disconnected",
    "VPN disconnected gracefully"
);

// Force kill required
warn!(
    pid = %pid,
    method = "SIGKILL",
    reason = "Graceful shutdown timeout",
    state = "Disconnected",
    "VPN force-killed"
);

// Disconnection failed
error!(
    pid = %pid,
    error = %error_msg,
    state = "Failed",
    "VPN disconnection failed"
);
```

## Testing

### Unit Test Contract

```rust
#[tokio::test]
async fn test_off_command_graceful_disconnect() {
    // Setup: Create mock state file
    let mock_state = ConnectionState {
        ip: "10.0.1.100".parse().unwrap(),
        device: "tun0".to_string(),
        connected_at: chrono::Utc::now(),
        pid: Some(12345),
    };
    save_mock_state(&mock_state);

    // Mock process that exits on SIGTERM
    let mock_process = MockProcess::new(12345)
        .expect_signal(Signal::SIGTERM)
        .exits_after(Duration::from_millis(100));

    // Execute
    let result = off_command().await;

    // Assert
    assert!(result.is_ok());
    assert!(!state_file_exists());

    // Verify SIGTERM was sent (not SIGKILL)
    mock_process.assert_signal_sent(Signal::SIGTERM);
    mock_process.assert_signal_not_sent(Signal::SIGKILL);
}

#[tokio::test]
async fn test_off_command_force_kill() {
    // Mock process that ignores SIGTERM
    let mock_process = MockProcess::new(12345)
        .expect_signal(Signal::SIGTERM)
        .ignores_signal()  // Doesn't exit
        .expect_signal(Signal::SIGKILL)
        .exits_immediately();

    let result = off_command().await;

    assert!(result.is_ok());
    mock_process.assert_signal_sent(Signal::SIGTERM);
    mock_process.assert_signal_sent(Signal::SIGKILL);
}

#[tokio::test]
async fn test_off_command_no_connection() {
    // No state file exists
    ensure_no_state_file();

    let result = off_command().await;

    assert!(matches!(result, Err(AkonError::StateError(_))));
}

#[tokio::test]
async fn test_off_command_stale_state() {
    // State file exists but process is gone
    let mock_state = ConnectionState {
        ip: "10.0.1.100".parse().unwrap(),
        device: "tun0".to_string(),
        connected_at: chrono::Utc::now(),
        pid: Some(99999),  // Non-existent PID
    };
    save_mock_state(&mock_state);

    let result = off_command().await;

    // Should clean up and succeed
    assert!(result.is_ok());
    assert!(!state_file_exists());
}
```

### Integration Test Contract

```rust
#[tokio::test]
#[ignore] // Requires real VPN connection
async fn test_real_vpn_disconnect() {
    // Setup: Establish real connection first
    establish_test_vpn_connection().await;

    // Execute disconnect
    let result = off_command().await;

    // Assert
    assert!(result.is_ok());

    // Verify no process running
    assert!(!vpn_process_running());

    // Verify no TUN device
    assert!(!tun_device_exists("tun0"));
}
```

## Edge Cases

1. **Process Already Dead (Stale State)**:
   - Detect via `kill(pid, 0)` returning ESRCH
   - Clean up state file without error
   - Inform user: "ℹ VPN already disconnected"

2. **No PID in State File**:
   - Older state format or manual state edit
   - Clean up state file
   - Warn user: "⚠ No process ID tracked"

3. **Permission Denied (Non-Root User)**:
   - `kill()` returns EPERM
   - Error: "Permission denied to terminate VPN process. Run with sudo."
   - State file left intact (connection still active)

4. **Unkillable Process (Kernel State)**:
   - Very rare: process in uninterruptible sleep (D state)
   - SIGKILL fails to terminate
   - Error: "Process could not be killed (may be in kernel state). Contact support."
   - State file cleaned anyway (manual intervention required)

5. **Multiple Disconnect Attempts**:
   - Race condition: two users run `akon vpn off` simultaneously
   - Second attempt finds no state file
   - Returns success (idempotent operation)

6. **Corrupted State File**:
   - JSON parse error
   - Clean up corrupted file
   - Error: "State file corrupted. VPN may still be running. Check manually with 'ip addr'."

## Dependencies

- `nix` crate (for signal handling, process management)
- `tokio::time::timeout` (for graceful shutdown timeout)
- `serde_json` (state file parsing)
- `tracing` (observability)
- `chrono` (connection duration tracking)

## System Interaction

### TUN Device Cleanup

OpenConnect automatically removes TUN device on exit. No manual cleanup required.

### Routing Table Cleanup

OpenConnect's `vpnc-script` handles route restoration on disconnect. No manual cleanup required.

## Backward Compatibility

- State file format may change (add `pid` field)
- Command syntax unchanged: `akon vpn off`
- Error messages improved but semantically equivalent

## Future Enhancements

1. **Graceful Reconnect**: Preserve connection state for quick reconnect
2. **Process Group Termination**: Kill entire OpenConnect process group
3. **Auto-Cleanup on Boot**: Systemd service to clean stale state files
4. **Connection Duration Stats**: Track total connection time before logging
5. **Multi-Connection Support**: Track multiple VPN profiles/connections
