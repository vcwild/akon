# Function Contracts: Fix VPN Off Command Cleanup

**Feature**: 004-fix-vpn-off
**Date**: 2025-11-08
**Phase**: 1 - Design

## Overview

This document defines the behavioral contracts for modified and newly tested functions. Since this is a bug fix that leverages existing code, most contracts are documented for clarity rather than defining new interfaces.

---

## Modified Functions

### `run_vpn_off()` - Enhanced VPN Disconnect Command

**Location**: `src/cli/vpn.rs`

**Signature**:

```rust
pub async fn run_vpn_off() -> Result<(), AkonError>
```

**Purpose**: Disconnect from VPN by terminating the tracked OpenConnect process and cleaning up any orphaned OpenConnect processes.

**Pre-conditions**:

- Function is called from CLI command `akon vpn off`
- Caller has permission to read/write `/tmp/akon_vpn_state.json`
- System utilities `ps`, `pgrep`, `kill` are available
- May require sudo privileges to terminate root-owned OpenConnect processes

**Post-conditions** (Success):

- All OpenConnect processes terminated (tracked + orphaned)
- State file removed from `/tmp/akon_vpn_state.json`
- User receives success message with termination details
- Returns `Ok(())`

**Post-conditions** (Partial Success):

- Tracked process terminated (if it existed)
- Some orphaned processes may remain due to permission errors
- State file removed
- User receives warning about processes that couldn't be terminated
- Returns `Ok(())` (partial success is acceptable)

**Post-conditions** (Failure):

- Critical error occurred (e.g., pgrep command failed)
- State may be inconsistent
- User receives error message with troubleshooting suggestions
- Returns `Err(AkonError)`

**Behavior**:

1. **Check for active connection**:
   - If state file doesn't exist → Print "No active connection" → Skip to step 5
   - If state file exists → Read and parse JSON

2. **Terminate tracked process**:
   - Extract PID from state file
   - Check if process is running (via `ps -p PID`)
   - If running:
     - Send SIGTERM via `sudo kill -TERM PID`
     - Poll every 500ms for up to 5 seconds (10 attempts)
     - If still running after timeout: Send SIGKILL via `sudo kill -KILL PID`
     - Wait 500ms for SIGKILL to take effect
   - Print status message (graceful or forced termination)

3. **Remove state file**:
   - Delete `/tmp/akon_vpn_state.json`
   - Log any deletion errors but don't fail

4. **Clean up orphaned processes**:
   - Call `cleanup_orphaned_processes()`
   - Handle result:
     - `Ok(0)`: Print "No orphaned processes found"
     - `Ok(count)`: Print "Terminated {count} process(es)"
     - `Err(e)`: Print warning but don't fail command

5. **Report completion**:
   - Print "Cleanup complete" message
   - Return `Ok(())`

**Error Handling**:

| Error Condition | Handling | Return Value |
|----------------|----------|--------------|
| State file missing | Continue to orphan cleanup | `Ok(())` |
| State file read/parse error | Log warning, continue to orphan cleanup | `Ok(())` |
| PID extraction failure | Log error, continue to orphan cleanup | `Ok(())` |
| SIGTERM send failure | Log error, attempt SIGKILL | `Ok(())` |
| SIGKILL send failure | Log error, continue to orphan cleanup | `Ok(())` |
| State file deletion failure | Log warning, continue | `Ok(())` |
| Orphan cleanup failure | Log warning, return success | `Ok(())` |
| Critical pgrep failure | Log error, fail command | `Err(AkonError)` |

**Side Effects**:

- Terminates system processes (OpenConnect)
- Removes state file from filesystem
- Logs to systemd journal via `tracing` crate
- Prints formatted output to stdout/stderr

**Thread Safety**: Not applicable (CLI function, single-threaded execution context)

**Async Behavior**: Uses `tokio::time::sleep()` for non-blocking polling delays

---

## Referenced Functions (No Changes)

### `cleanup_orphaned_processes()` - Comprehensive Process Cleanup

**Location**: `src/daemon/process.rs:192-303`

**Signature**:

```rust
pub fn cleanup_orphaned_processes() -> Result<usize, AkonError>
```

**Purpose**: Find and terminate all OpenConnect processes on the system, regardless of how they were started.

**Pre-conditions**:

- `pgrep` utility available on system
- Caller has permission to signal processes (may require elevated privileges)

**Post-conditions** (Success):

- All accessible OpenConnect processes terminated
- Returns `Ok(count)` where `count` is number of terminated processes
- Processes owned by other users that can't be terminated are logged but skipped

**Post-conditions** (Failure):

- Unable to search for processes (pgrep command failed)
- Returns `Err(AkonError::Vpn(VpnError::ConnectionFailed { reason }))`

**Behavior**:

1. Run `pgrep -x openconnect` to find all matching processes
2. Parse PIDs from output
3. For each PID:
   - Send SIGTERM
   - Wait 5 seconds
   - Check if still running (via `kill(pid, None)`)
   - If still running: Send SIGKILL
   - Handle permission errors gracefully (skip process, log warning)
4. Return count of successfully terminated processes

**Error Handling**:

- `ESRCH` (process not found): Count as successful, already terminated
- `EPERM` (permission denied): Log warning, skip to next process
- Other signal errors: Log warning, attempt next step

**No modifications needed** - function already provides required behavior.

---

## Deprecated Functions

### `run_vpn_cleanup()` - Legacy Cleanup Command (To Be Deprecated)

**Location**: `src/cli/vpn.rs:1184-1230`

**Current Behavior**: Calls `cleanup_orphaned_processes()` and removes state file

**Future Behavior** (Phase 1 - This Feature):

- Add deprecation warning at function start
- Internally delegate to `run_vpn_off()`
- Maintain return signature for compatibility

**Deprecation Plan**:

```rust
pub async fn run_vpn_cleanup() -> Result<(), AkonError> {
    println!(
        "{} {}",
        "⚠".bright_yellow(),
        "DEPRECATED: 'akon vpn cleanup' is deprecated. Use 'akon vpn off' instead.".bright_yellow()
    );
    println!(
        "  This command will be removed in a future version."
    );
    println!();

    // Delegate to run_vpn_off
    run_vpn_off().await
}
```

**Removal Timeline**: Phase 2 (future feature, not part of this implementation)

---

## Testing Contracts

### Test: Comprehensive Cleanup After Disconnect

**File**: `tests/vpn_disconnect_tests.rs`

**Purpose**: Verify that `run_vpn_off()` terminates both tracked and orphaned processes

**Setup**:

1. Spawn 3 test OpenConnect processes (using dummy script or actual openconnect with test args)
2. Create state file with PID of one process (tracked)
3. Leave two processes untracked (orphaned)

**Execution**:

1. Call `run_vpn_off()`
2. Wait for completion

**Assertions**:

- State file removed
- All 3 OpenConnect processes terminated
- `pgrep -x openconnect` returns no results
- Function returned `Ok(())`

**Teardown**:

- Kill any remaining test processes
- Clean up state file if it exists

---

### Test: Graceful Handling of No Active Connection

**File**: `tests/vpn_disconnect_tests.rs`

**Purpose**: Verify that `run_vpn_off()` handles missing state file gracefully

**Setup**:

1. Ensure no state file exists
2. Spawn 2 orphaned OpenConnect test processes

**Execution**:

1. Call `run_vpn_off()`
2. Wait for completion

**Assertions**:

- Function returned `Ok(())`
- Both orphaned processes terminated
- User sees "No active connection" message
- Cleanup still runs and reports success

**Teardown**:

- Kill any remaining test processes

---

### Test: Permission Error Handling

**File**: `tests/vpn_disconnect_tests.rs`

**Purpose**: Verify graceful handling when permissions are insufficient

**Setup**:

1. Spawn test process owned by different user (requires multi-user test environment or mock)
2. Create state file pointing to inaccessible process

**Execution**:

1. Call `run_vpn_off()` without sudo
2. Wait for completion

**Assertions**:

- Function returned `Ok(())` (doesn't fail on permission errors)
- Warning message logged about permission denied
- User-owned processes (if any) are terminated
- State file removed despite permission errors

**Teardown**:

- Clean up any test processes with sudo

---

## Integration Points

### Command Line Interface

**Entry Point**: `src/main.rs` via clap command routing

**Command**: `akon vpn off`

**Arguments**: None

**Exit Codes**:

- `0`: Success (all cleanup completed or partial success with warnings)
- `1`: Critical failure (pgrep command failed, system error)

**Output Format**:

```text
🔌 Disconnecting VPN (PID: 12345)...
✓ VPN disconnected gracefully
🧹 Cleaning up orphaned OpenConnect processes...
✓ Terminated 2 process(es)
✓ Cleanup complete
```

### State File Integration

**Read Operations**:

- Location: `/tmp/akon_vpn_state.json`
- Format: JSON with `pid`, `ip`, `device`, `connected_at` fields
- Error handling: Missing or corrupt file treated as "no active connection"

**Write Operations**:

- Delete state file after successful disconnect
- Log warning if deletion fails but don't fail command

### Logging Integration

**Log Levels**:

- `DEBUG`: Process polling, signal attempts, timing details
- `INFO`: State transitions, cleanup start/complete, process counts
- `WARN`: Permission errors, signal failures, state file issues
- `ERROR`: Critical failures (pgrep command failure)

**Log Format**: Structured logging via `tracing` crate, outputs to systemd journal

---

## Backward Compatibility

### State File Format

**No changes**: Existing state file format remains unchanged. This ensures:

- Existing VPN connections can be disconnected after upgrade
- No migration needed
- Rollback to previous version is safe

### Command Interface

**Maintained**:

- `akon vpn off` continues to work as before, with enhanced cleanup
- Exit codes unchanged
- Output format similar (added cleanup messages)

**Deprecated**:

- `akon vpn cleanup` will show deprecation warning but continue to work
- Users have time to update scripts before removal

---

## Summary

This contract specification defines:

- ✅ Enhanced `run_vpn_off()` behavior with comprehensive cleanup
- ✅ Clear pre/post-conditions and error handling strategies
- ✅ Integration with existing `cleanup_orphaned_processes()` function
- ✅ Deprecation path for `run_vpn_cleanup()` command
- ✅ Testing contracts to verify correctness
- ✅ Backward compatibility guarantees

The contracts ensure that the bug fix maintains system stability while improving reliability of the disconnect operation.
