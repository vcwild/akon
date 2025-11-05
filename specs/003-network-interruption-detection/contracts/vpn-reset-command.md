# Contract: `akon vpn reset` Command

**Feature**: Network Interruption Detection and Automatic Reconnection
**User Story**: US4 - Manual Process Cleanup and Reset
**Task**: T054

## Overview

The `vpn reset` command clears reconnection error states and retry counters, allowing the system to attempt reconnection after max attempts have been exceeded.

## Command Signature

```bash
akon vpn reset
```

## Purpose

- Clear retry counter after max attempts exceeded
- Reset consecutive failure counter
- Transition from Error state to Disconnected state
- Prepare system for new connection attempts

## Behavior

### State Reset

1. **Clear retry counter**: Reset internal attempt counter to 0
2. **Clear failure counter**: Reset consecutive health check failures to 0
3. **Clear error state**: Transition from Error to Disconnected
4. **Remove state file**: Clear cached connection state

### Integration Points

- **When fully integrated**: Send `ReconnectionCommand::ResetRetries` to ReconnectionManager via IPC
- **Current workaround**: Clear state file and provide manual steps

## Output Examples

### Standard Output (Current Implementation)

```
🔄 Resetting reconnection state...
  ℹ This feature requires integration with the VPN daemon
  Workaround: Disconnect and reconnect:
    1. akon vpn off
    2. akon vpn cleanup
    3. akon vpn on
  ✓ Cleared connection state
✓ Reset complete - ready for new connection attempt
```

**Exit Code**: 0

### Future Fully Integrated Output

```
🔄 Resetting reconnection state...
  ✓ Cleared retry counter (was 5)
  ✓ Cleared consecutive failure counter (was 3)
  ✓ Transitioned from Error to Disconnected
✓ Reset complete - ready for reconnection
```

**Exit Code**: 0

### When Not in Error State

```
🔄 Resetting reconnection state...
  ℹ System is not in error state
  Current state: Connected
  No reset needed
```

**Exit Code**: 0

## Requirements

### Functional

- **FR-1**: Must reset retry counter to 0
- **FR-2**: Must reset consecutive failure counter to 0
- **FR-3**: Must transition from Error state to Disconnected state
- **FR-4**: Must clear state file
- **FR-5**: Must log reset action
- **FR-6**: Must provide clear user feedback
- **FR-7**: Must be safe to call in any state (idempotent)

### Non-Functional

- **NFR-1**: Should complete within 1 second
- **NFR-2**: Should not require elevated privileges
- **NFR-3**: Should work when no active connection exists
- **NFR-4**: Should log all state transitions
- **NFR-5**: Should provide workaround until full integration

## State Transitions

```
Error → Disconnected     (primary use case)
Reconnecting → Disconnected (clears ongoing reconnection)
Connected → Connected    (no-op, already in good state)
Disconnected → Disconnected (no-op, already reset)
```

## Error Handling

| Error | Behavior | Exit Code |
|-------|----------|-----------|
| State file locked | Retry or display error | 1 |
| State file removal error | Log warning, continue | 0 |
| IPC unavailable (future) | Fall back to manual steps | 0 |
| Permission denied | Display error with details | 1 |

## Security Considerations

1. **State file access**: Only modify own user's state file
2. **No privilege escalation**: Never requires sudo
3. **Safe state transitions**: All transitions are valid
4. **Audit logging**: Log all reset actions

## Use Cases

### Use Case 1: Max Attempts Exceeded

**Scenario**: Automatic reconnection failed after 5 attempts

**Steps**:
1. User runs `akon vpn status` → sees Error state
2. User fixes network issue or checks VPN server
3. User runs `akon vpn reset`
4. User runs `akon vpn on` → connection succeeds

### Use Case 2: Stuck in Reconnecting Loop

**Scenario**: System is reconnecting but user wants to stop

**Steps**:
1. User runs `akon vpn reset`
2. System clears ongoing reconnection
3. User can now try manual connection

### Use Case 3: Fresh Start

**Scenario**: User wants to ensure clean state before connecting

**Steps**:
1. User runs `akon vpn reset`
2. User runs `akon vpn cleanup` (optional)
3. User runs `akon vpn on`

## Testing

### Test Cases

1. **Error state**: Verify transition to Disconnected
2. **Connected state**: Verify no-op behavior
3. **No state file**: Verify no error, graceful handling
4. **State file exists**: Verify removal
5. **Retry counter**: Verify reset to 0
6. **Failure counter**: Verify reset to 0
7. **Multiple resets**: Verify idempotent behavior

### Manual Testing

```bash
# 1. Force error state (simulate max attempts)
# (This would require running reconnection until failure)

# 2. Run reset
akon vpn reset

# 3. Verify state cleared
akon vpn status  # Should show "Not connected"

# 4. Verify can reconnect
akon vpn on
```

## Integration Points

### Current (Workaround Mode)

- **State File**: Removes `/tmp/akon_vpn_state.json`
- **CLI Output**: Provides manual recovery steps

### Future (Full Integration)

- **IPC Channel**: Sends command to ReconnectionManager
- **State Machine**: Directly updates internal state
- **Logging**: Records state transitions
- **Status Reporting**: Returns detailed reset information

## Dependencies

- File system: Write access to state file
- **Future**: IPC mechanism to communicate with daemon
- **Future**: ReconnectionManager command handling

## Future Enhancements

1. **IPC Integration**: Send command to running daemon
2. **Detailed reporting**: Show exact counters before/after reset
3. **Selective reset**: Options to reset only specific counters
4. **Auto-reconnect**: Optional flag to reconnect after reset
5. **Confirmation prompt**: Interactive mode for safety
