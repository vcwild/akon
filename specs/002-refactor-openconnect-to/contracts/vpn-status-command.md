# Contract: VPN Status Command

**Feature**: 002-refactor-openconnect-to
**Module**: `src/cli/vpn.rs::status_command()`
**Purpose**: Display current VPN connection status with human-readable and machine-parsable output

## Interface

### Function Signature

```rust
pub fn status_command() -> Result<(), AkonError>
```

### Parameters

None (reads from state file)

### Returns

- `Ok(())` - Status displayed successfully
- `Err(AkonError)` - Error reading state (displays as "Not connected")

### Exit Codes

- `0` - VPN connected and active
- `1` - VPN not connected
- `2` - State error (corrupted state file, process dead but state exists)

## Behavior

### Pre-conditions

None (command always succeeds, displays appropriate status)

### Post-conditions

1. User sees connection status (connected/disconnected)
2. If connected: IP, device, duration, PID displayed
3. If not connected: Clear "Not connected" message
4. Exit code reflects connection state

## Implementation Contract

### Step 1: Load Connection State

```rust
use std::fs;
use serde_json;

fn load_connection_state() -> Option<ConnectionState> {
    let state_path = "/tmp/akon_vpn_state.json";

    if !std::path::Path::new(state_path).exists() {
        return None;
    }

    let state_json = match fs::read_to_string(state_path) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to read state file");
            return None;
        }
    };

    match serde_json::from_str::<ConnectionState>(&state_json) {
        Ok(state) => Some(state),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse state file");
            None
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ConnectionState {
    ip: std::net::IpAddr,
    device: String,
    connected_at: chrono::DateTime<chrono::Utc>,
    pid: Option<u32>,
}
```

### Step 2: Verify Process Still Running (if PID available)

```rust
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

fn is_process_running(pid: u32) -> bool {
    match kill(Pid::from_raw(pid as i32), Signal::SIGNULL) {
        Ok(_) => true,
        Err(_) => false,
    }
}
```

### Step 3: Display Status (Human-Readable)

```rust
pub fn status_command() -> Result<(), AkonError> {
    match load_connection_state() {
        Some(state) => {
            // Check if process still running (if PID available)
            let process_running = state.pid
                .map(|pid| is_process_running(pid))
                .unwrap_or(true);  // Assume running if no PID tracked

            if process_running {
                display_connected_status(&state);
                std::process::exit(0);
            } else {
                display_stale_state_warning(&state);
                std::process::exit(2);  // Stale state
            }
        }
        None => {
            display_not_connected();
            std::process::exit(1);
        }
    }
}

fn display_connected_status(state: &ConnectionState) {
    println!("✓ VPN Status: Connected");
    println!();
    println!("  IP Address:  {}", state.ip);
    println!("  Device:      {}", state.device);

    let duration = chrono::Utc::now() - state.connected_at;
    println!("  Connected:   {}", format_duration(duration));

    if let Some(pid) = state.pid {
        println!("  Process ID:  {}", pid);
    }

    tracing::debug!(
        ip = %state.ip,
        device = %state.device,
        duration_seconds = %duration.num_seconds(),
        "VPN status checked (connected)"
    );
}

fn display_not_connected() {
    println!("✗ VPN Status: Not connected");
    println!();
    println!("  Run 'akon vpn on' to connect");

    tracing::debug!("VPN status checked (not connected)");
}

fn display_stale_state_warning(state: &ConnectionState) {
    println!("⚠ VPN Status: Stale state detected");
    println!();
    println!("  Last known IP:  {}", state.ip);
    println!("  Last device:    {}", state.device);
    println!("  Disconnected:   {} ago", format_duration(chrono::Utc::now() - state.connected_at));

    if let Some(pid) = state.pid {
        println!("  Process {} is no longer running", pid);
    }

    println!();
    println!("  Run 'akon vpn off' to clean up stale state");

    tracing::warn!(
        pid = ?state.pid,
        "Stale VPN state detected (process not running)"
    );
}

fn format_duration(duration: chrono::Duration) -> String {
    let seconds = duration.num_seconds();

    if seconds < 60 {
        format!("{} seconds", seconds)
    } else if seconds < 3600 {
        format!("{} minutes", seconds / 60)
    } else if seconds < 86400 {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        format!("{} hours, {} minutes", hours, minutes)
    } else {
        let days = seconds / 86400;
        let hours = (seconds % 86400) / 3600;
        format!("{} days, {} hours", days, hours)
    }
}
```

## Output Format

### Connected State

```
✓ VPN Status: Connected

  IP Address:  10.0.1.100
  Device:      tun0
  Connected:   2 hours, 15 minutes
  Process ID:  12345
```

### Not Connected

```
✗ VPN Status: Not connected

  Run 'akon vpn on' to connect
```

### Stale State

```
⚠ VPN Status: Stale state detected

  Last known IP:  10.0.1.100
  Last device:    tun0
  Disconnected:   5 minutes ago
  Process 12345 is no longer running

  Run 'akon vpn off' to clean up stale state
```

## Machine-Parsable Output (Future: JSON Format)

**Future Enhancement**: Add `--json` flag for machine-readable output:

```bash
$ akon vpn status --json
```

```json
{
  "status": "connected",
  "ip": "10.0.1.100",
  "device": "tun0",
  "connected_at": "2025-11-03T10:30:00Z",
  "duration_seconds": 8100,
  "pid": 12345
}
```

```json
{
  "status": "not_connected"
}
```

```json
{
  "status": "stale_state",
  "last_ip": "10.0.1.100",
  "last_device": "tun0",
  "disconnected_at": "2025-11-03T10:30:00Z",
  "pid": 12345
}
```

## Performance Requirements

- **Execution Time**: <100ms (file read + process check)
- **No Network Calls**: All information from local state
- **No Elevated Privileges**: Runs without sudo

## Security Requirements

1. **Read-Only Operation**: No state modifications
2. **No Sensitive Data Display**: IP address is non-sensitive metadata
3. **State File Permissions**: Verify readable by user

## Observability (FR-022)

```rust
use tracing::{debug, warn};

// Status check (connected)
debug!(
    ip = %state.ip,
    device = %state.device,
    duration_seconds = %duration.num_seconds(),
    "VPN status checked (connected)"
);

// Status check (not connected)
debug!("VPN status checked (not connected)");

// Stale state detected
warn!(
    pid = ?state.pid,
    last_ip = %state.ip,
    "Stale VPN state detected"
);
```

## Testing

### Unit Test Contract

```rust
#[test]
fn test_status_command_connected() {
    // Setup: Create mock state file with recent connection
    let mock_state = ConnectionState {
        ip: "10.0.1.100".parse().unwrap(),
        device: "tun0".to_string(),
        connected_at: chrono::Utc::now() - chrono::Duration::minutes(10),
        pid: Some(12345),
    };
    save_mock_state(&mock_state);

    // Mock process is running
    MockProcess::set_running(12345, true);

    // Execute (captures stdout)
    let output = run_status_command();

    // Assert
    assert!(output.contains("✓ VPN Status: Connected"));
    assert!(output.contains("10.0.1.100"));
    assert!(output.contains("tun0"));
    assert!(output.contains("10 minutes"));
    assert_exit_code(0);
}

#[test]
fn test_status_command_not_connected() {
    // No state file
    ensure_no_state_file();

    let output = run_status_command();

    assert!(output.contains("✗ VPN Status: Not connected"));
    assert_exit_code(1);
}

#[test]
fn test_status_command_stale_state() {
    // State file exists but process is gone
    let mock_state = ConnectionState {
        ip: "10.0.1.100".parse().unwrap(),
        device: "tun0".to_string(),
        connected_at: chrono::Utc::now() - chrono::Duration::hours(2),
        pid: Some(99999),  // Non-existent PID
    };
    save_mock_state(&mock_state);

    // Mock process not running
    MockProcess::set_running(99999, false);

    let output = run_status_command();

    assert!(output.contains("⚠ VPN Status: Stale state"));
    assert!(output.contains("no longer running"));
    assert_exit_code(2);
}

#[test]
fn test_format_duration() {
    use chrono::Duration;

    assert_eq!(format_duration(Duration::seconds(30)), "30 seconds");
    assert_eq!(format_duration(Duration::seconds(90)), "1 minutes");
    assert_eq!(format_duration(Duration::seconds(3600)), "1 hours, 0 minutes");
    assert_eq!(format_duration(Duration::seconds(7320)), "2 hours, 2 minutes");
    assert_eq!(format_duration(Duration::seconds(90000)), "1 days, 1 hours");
}

#[test]
fn test_status_no_pid_tracked() {
    // Legacy state format without PID
    let mock_state = ConnectionState {
        ip: "10.0.1.100".parse().unwrap(),
        device: "tun0".to_string(),
        connected_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        pid: None,  // No PID tracked
    };
    save_mock_state(&mock_state);

    let output = run_status_command();

    // Should assume connected (can't verify process)
    assert!(output.contains("✓ VPN Status: Connected"));
    assert!(!output.contains("Process ID:"));  // PID not displayed
    assert_exit_code(0);
}
```

### Integration Test Contract

```rust
#[test]
#[ignore] // Requires real VPN connection
fn test_status_with_real_connection() {
    // Setup: Establish real VPN connection
    establish_test_vpn_connection();

    // Wait for connection
    std::thread::sleep(std::time::Duration::from_secs(5));

    // Execute status check
    let output = run_command("akon", &["vpn", "status"]);

    // Assert connected
    assert!(output.contains("Connected"));
    assert_exit_code(0);

    // Cleanup
    disconnect_vpn();
}
```

## Edge Cases

1. **State File Corrupted (Invalid JSON)**:
   - Parse error caught
   - Treated as "Not connected"
   - Exit code: 1
   - Warning logged

2. **State File Unreadable (Permissions)**:
   - Read error caught
   - Treated as "Not connected"
   - Exit code: 1

3. **Duration Overflow (Very Long Connection)**:
   - Use `chrono::Duration::num_seconds()` (handles large values)
   - Display days for connections >24 hours

4. **Negative Duration (Clock Skew)**:
   - Handle clock adjustment gracefully
   - Display "just now" for negative durations

5. **Multiple Status Checks (Concurrent)**:
   - Read-only operation, no race conditions
   - Each check independent

6. **Status Check During Connection**:
   - May show "Connecting" state (if state file created early)
   - Future enhancement: add `status` field to state file

## Dependencies

- `nix` crate (for process existence check)
- `serde_json` (state file parsing)
- `chrono` (duration calculations)
- `tracing` (observability)

## Backward Compatibility

- State file format unchanged (backward compatible)
- Command syntax unchanged: `akon vpn status`
- Output format improved (more information)

## Future Enhancements

1. **JSON Output** (`--json` flag):
   - Machine-parsable format for scripting
   - Include all connection metadata

2. **Live Statistics** (`--live` flag):
   - Real-time bandwidth monitoring
   - Parse OpenConnect's periodic statistics

3. **Connection Quality Metrics**:
   - Latency to VPN gateway
   - Packet loss statistics
   - Jitter measurements

4. **Historical Connection Stats**:
   - Track connection history (timestamps, durations)
   - Display last N connections

5. **Health Check**:
   - Test connectivity through VPN
   - Verify DNS resolution
   - Check split tunnel configuration

6. **Status in State Field**:
   - Add `"status": "connecting" | "authenticating" | "connected"` to state file
   - Display more granular status during connection

## Integration with Other Commands

### `vpn on` → `vpn status`

After successful connection:
```bash
$ sudo akon vpn on
Connecting to VPN server vpn.example.com...
  Authenticating...
  ✓ F5 session established
  ✓ TUN device tun0 configured
✓ Connected to VPN
  IP Address: 10.0.1.100
  Device: tun0

$ akon vpn status  # No sudo required
✓ VPN Status: Connected

  IP Address:  10.0.1.100
  Device:      tun0
  Connected:   just now
  Process ID:  12345
```

### `vpn off` → `vpn status`

After disconnection:
```bash
$ sudo akon vpn off
Disconnecting VPN...
✓ VPN disconnected gracefully

$ akon vpn status
✗ VPN Status: Not connected

  Run 'akon vpn on' to connect
```

### Stale State Detection

If OpenConnect crashes:
```bash
$ akon vpn status
⚠ VPN Status: Stale state detected

  Last known IP:  10.0.1.100
  Last device:    tun0
  Disconnected:   5 minutes ago
  Process 12345 is no longer running

  Run 'akon vpn off' to clean up stale state

$ sudo akon vpn off
ℹ OpenConnect process not running (already disconnected?)
✓ State cleaned up

$ akon vpn status
✗ VPN Status: Not connected

  Run 'akon vpn on' to connect
```

## Exit Code Usage in Scripts

```bash
#!/bin/bash

# Check if VPN is connected before running sensitive operation
if akon vpn status > /dev/null 2>&1; then
    echo "VPN connected, proceeding..."
    # Do sensitive work
else
    echo "VPN not connected! Connecting..."
    sudo akon vpn on
fi
```

```bash
# Monitor VPN status
while true; do
    if ! akon vpn status > /dev/null 2>&1; then
        echo "VPN disconnected! Reconnecting..."
        sudo akon vpn on
    fi
    sleep 60
done
```
