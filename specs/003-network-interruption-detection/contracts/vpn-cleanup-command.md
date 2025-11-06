# Contract: `akon vpn cleanup` Command

**Feature**: Network Interruption Detection and Automatic Reconnection
**User Story**: US4 - Manual Process Cleanup and Reset
**Task**: T054

## Overview

The `vpn cleanup` command provides a manual way to terminate orphaned OpenConnect processes when automatic reconnection fails or processes become stuck.

## Command Signature

```bash
akon vpn cleanup
```

## Purpose

- Terminate all orphaned OpenConnect processes
- Clean up stale VPN connections
- Reset system to a clean state for reconnection attempts
- Provide user feedback on cleanup actions

## Behavior

### Process Discovery

1. Search for all processes named `openconnect`
2. Use `pgrep -x openconnect` for exact matching
3. Collect all matching process IDs (PIDs)

### Graceful Termination

For each discovered process:

1. **Send SIGTERM**: Attempt graceful shutdown
2. **Wait 5 seconds**: Give process time to clean up
3. **Check status**: Verify if process terminated
4. **Send SIGKILL**: Force termination if still running
5. **Handle errors**: Log permission denied and other errors

### State Cleanup

After process termination:

1. Remove VPN state file (`/tmp/akon_vpn_state.json`)
2. Update internal connection state to `Disconnected`
3. Clear any cached connection information

## Output Examples

### No Processes Found

```
🧹 Cleaning up orphaned OpenConnect processes...
  ✓ No orphaned processes found
✓ Cleanup complete
```

**Exit Code**: 0

### Processes Terminated

```
🧹 Cleaning up orphaned OpenConnect processes...
  ✓ Terminated 2 process(es)
✓ Cleanup complete
```

**Exit Code**: 0

### Permission Errors

```
🧹 Cleaning up orphaned OpenConnect processes...
✗ Cleanup failed
  Error: Permission denied

💡 Suggestions:
   • Check for permission issues (may need sudo)
   • Manually check processes: ps aux | grep openconnect
   • Review system logs: journalctl -xe
```

**Exit Code**: 1

## Requirements

### Functional

- **FR-1**: Must find all OpenConnect processes regardless of owner
- **FR-2**: Must send SIGTERM before SIGKILL
- **FR-3**: Must wait at least 5 seconds between SIGTERM and SIGKILL
- **FR-4**: Must handle permission errors gracefully (log warning, continue with other processes)
- **FR-5**: Must remove state file after successful cleanup
- **FR-6**: Must return count of terminated processes
- **FR-7**: Must provide clear user feedback for all outcomes

### Non-Functional

- **NFR-1**: Should complete within 10 seconds for up to 5 processes
- **NFR-2**: Should be safe to run multiple times (idempotent)
- **NFR-3**: Should not fail if no processes exist
- **NFR-4**: Should log all actions for troubleshooting
- **NFR-5**: Should handle partial failures (some processes terminate, others don't)

## Error Handling

| Error | Behavior | Exit Code |
|-------|----------|-----------|
| No pgrep command | Display error, suggest installation | 1 |
| Permission denied (single process) | Log warning, continue with others | 0 (if others succeed) |
| Permission denied (all processes) | Display error with sudo suggestion | 1 |
| Process not found (ESRCH) | Count as terminated, continue | 0 |
| State file removal error | Log warning, continue | 0 |

## Security Considerations

1. **Process identification**: Only target processes named exactly "openconnect"
2. **Permission handling**: Never escalate privileges automatically
3. **Signal safety**: Use standard Unix signals (SIGTERM, SIGKILL)
4. **State file**: Remove only if cleanup successful

## Testing

### Test Cases

1. **No processes**: Verify message and exit code 0
2. **Single process**: Verify termination and count
3. **Multiple processes**: Verify all terminated
4. **SIGTERM responsive**: Verify process terminates without SIGKILL
5. **SIGTERM unresponsive**: Verify SIGKILL sent after 5 seconds
6. **Permission denied**: Verify error handling and suggestions
7. **State file exists**: Verify removal after cleanup
8. **State file missing**: Verify no error

### Manual Testing

```bash
# 1. Spawn test process
sleep 3600 &
MOCK_PID=$!

# 2. Run cleanup (will not find 'openconnect' named processes)
akon vpn cleanup

# 3. Test with actual openconnect (requires sudo)
# sudo openconnect --background vpn.example.com &
# akon vpn cleanup  # Should terminate it
```

## Integration Points

- **VPN State Management**: Updates connection state to Disconnected
- **Logging System**: Logs all cleanup actions and errors
- **Process Management**: Uses system signal handling
- **CLI Framework**: Returns appropriate exit codes

## Dependencies

- System commands: `pgrep`
- Rust crates: `nix` (for signal handling)
- File system: Write access to state file location

## Future Enhancements

1. **Selective cleanup**: Option to specify PIDs to terminate
2. **Force mode**: Skip graceful termination, go straight to SIGKILL
3. **Dry run**: Show what would be cleaned up without doing it
4. **Batch operations**: Cleanup multiple VPN types (not just OpenConnect)
