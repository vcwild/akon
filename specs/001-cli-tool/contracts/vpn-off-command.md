# CLI Contract: VPN Off Command

**Command**: `akon vpn off`
**Priority**: P1 (User Story 4)
**Purpose**: Gracefully disconnect VPN connection

## Signature

```bash
akon vpn off [OPTIONS]
```

## Options

| Flag | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `--force` | Flag | Yes | false | Force kill daemon if graceful shutdown fails |
| `--timeout` | `u64` | No | 10 | Shutdown timeout in seconds |

## Behavior

1. **Connection Check**:
   - Check if daemon is running (PID file: `/tmp/akon-daemon.pid`)
   - If not running: Display "Already disconnected" and exit 0 (idempotent)
   - If PID file exists but process dead: Clean up stale files, exit 0

2. **Graceful Shutdown Request**:
   - Connect to Unix socket (`/tmp/akon-daemon.sock`)
   - Send `IpcMessage::Shutdown`
   - Wait for daemon exit (poll PID, timeout: 10 seconds)

3. **Daemon Shutdown** (daemon process):
   - Receive shutdown signal via IPC
   - Call `openconnect_close()` to tear down tunnel
   - Clean up resources (socket, PID file, state file)
   - Log event: `event=vpn_disconnected reason=user_requested`
   - Exit 0

4. **Force Shutdown** (if graceful fails):
   - If `--force` flag: Send SIGKILL to daemon PID
   - If no `--force`: Display error and exit 1
   - Clean up stale files manually

5. **Cleanup Verification**:
   - Verify PID file removed
   - Verify Unix socket removed
   - Verify VPN interface down (e.g., `tun0` no longer exists)

6. **Display Success**:
   - Show disconnection message with uptime
   - Exit 0

## Exit Codes

- `0`: VPN disconnected successfully (or already disconnected)
- `1`: Shutdown failed (daemon unresponsive, permission denied)
- `2`: Internal error (IPC failure, file system issue)

## Output Examples

### Success

```
🔌 Disconnecting from VPN...
✅ VPN disconnected successfully

Session duration: 3h 42m 15s
```

### Already Disconnected

```
✅ VPN already disconnected

Run `akon vpn on` to connect.
```

### Graceful Shutdown Failed (No --force)

```
🔌 Disconnecting from VPN...

⚠️  Warning: Daemon did not respond to shutdown request

Options:
  - Retry with --force flag to kill daemon process
  - Check logs: journalctl -t akon -n 50

Exit code: 1
```

### Force Shutdown

```
🔌 Disconnecting from VPN...
⚠️  Daemon unresponsive, forcing shutdown...
✅ VPN disconnected (forced)

Note: Check logs for connection errors: journalctl -t akon -n 50
```

### Permission Denied

```
❌ Error: Permission denied

Cannot terminate daemon process (PID: 12345).
Try running with elevated privileges if needed.

Exit code: 1
```

## State Changes

- Terminates daemon process
- Removes PID file (`/tmp/akon-daemon.pid`)
- Removes Unix socket (`/tmp/akon-daemon.sock`)
- Removes state file (`/tmp/akon-state.json`)
- Tears down VPN tunnel interface (e.g., `tun0`)
- Logs events:
  - `event=shutdown_requested`
  - `event=vpn_disconnected uptime=<seconds>`
  - `event=cleanup_complete`

## Security Requirements

- Only user who created daemon can terminate it (Unix permissions)
- SIGKILL should be last resort (prefer SIGTERM for cleanup)
- Cleanup MUST remove IPC socket to prevent stale file issues

## Dependencies

- Daemon process running (or no-op if not running)
- Write access to `/tmp/` for file cleanup
- Permission to terminate daemon process

## Process Model

```
┌─────────────┐
│ CLI Process │
│ (akon vpn off)│
└──────┬──────┘
       │
       │ 1. Connect to socket
       │    /tmp/akon-daemon.sock
       │
       ▼
┌──────────────────────┐
│ Unix Socket          │
└──────────┬───────────┘
           │ IpcMessage::Shutdown
           │
           ▼
    ┌──────────────┐
    │ Daemon Process│
    │              │
    │ 1. Close VPN │
    │ 2. Cleanup   │
    │ 3. Exit 0    │
    └──────────────┘
```

## Test Scenarios

1. **Happy Path**: Daemon running → Send shutdown, wait for exit, verify cleanup, exit 0
2. **Already Disconnected**: No daemon running → Display message, exit 0
3. **Stale PID File**: PID file exists, process dead → Clean up files, exit 0
4. **Unresponsive Daemon**: No response to shutdown → Display error, exit 1
5. **Force Kill**: `--force` flag + unresponsive → SIGKILL, cleanup, exit 0
6. **Permission Denied**: Cannot kill daemon → Display error, exit 1
7. **IPC Failure**: Socket connection fails → Fallback to SIGTERM, exit 0
8. **Partial Cleanup**: Some files remain → Log warning, manual cleanup instructions

## Idempotency

- Running `akon vpn off` multiple times MUST be safe
- If already disconnected: Exit 0 with friendly message
- No side effects from repeated invocations

## Integration with Monitoring Service

- Monitoring service (User Story 5, P3) MUST respect user-initiated shutdown
- When `vpn off` is called, monitoring service MUST NOT attempt reconnection
- Implementation: Write flag file (`/tmp/akon-no-auto-reconnect`) before shutdown
- Monitoring service reads flag, skips reconnection logic

## Error Recovery

| Scenario | Recovery Action |
|----------|----------------|
| Socket connection timeout | Fallback to SIGTERM signal |
| Process not responding to SIGTERM | Require `--force` for SIGKILL |
| PID file removal fails | Log error, exit 1 |
| VPN interface still up after shutdown | Log error, suggest manual `ip link delete tun0` |
