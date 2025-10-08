# CLI Contract: VPN On Command

**Command**: `akon vpn on`
**Priority**: P1 (User Story 2, 4)
**Purpose**: Establish VPN connection with OTP authentication

## Signature

```bash
akon vpn on [OPTIONS]
```

## Options

| Flag | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `--config` | `Path` | No | `~/.config/akon/config.toml` | Override config file location |
| `--foreground` | Flag | No | false | Run in foreground (don't daemonize) |
| `--timeout` | `u64` | No | 30 | Connection timeout in seconds |

## Behavior

1. **Idempotency Check**:
   - Check if daemon already running (PID file: `/tmp/akon-daemon.pid`)
   - If already connected: Display "Already connected" and exit 0
   - If connecting: Wait for completion, then exit with result

2. **Configuration Load**:
   - Load `~/.config/akon/config.toml`
   - Validate required fields (server, username, protocol)
   - If invalid: Display error and exit 2

3. **Keyring Access**:
   - Retrieve OTP secret from GNOME Keyring (service: `akon-vpn-otp`)
   - If locked: Prompt to unlock, retry once
   - If not found: Display "Run `akon setup` first" and exit 2

4. **TOTP Generation**:
   - Generate current TOTP token from OTP secret
   - Validate token format (6-8 digits)
   - Log event: `event=otp_generated` (no token value)

5. **Daemon Spawn**:
   - Fork background process (unless `--foreground`)
   - Child: Establish OpenConnect connection via FFI
   - Parent: Block until connection signal received

6. **Connection Establishment** (child process):
   - Call `openconnect_new_from_url()` via FFI
   - Set auth callback with TOTP token
   - Call `openconnect_obtain_cookie()` to authenticate
   - Call `openconnect_make_cstp_connection()` to establish tunnel
   - On success: Signal parent via Unix socket, continue running
   - On failure: Signal parent with error, exit 1

7. **Parent Wait**:
   - Listen on Unix socket (`/tmp/akon-daemon.sock`)
   - Wait for connection signal (timeout: 30 seconds)
   - On success: Display connection details, exit 0
   - On timeout: Kill child, exit 1
   - On failure: Display error category, exit 1

8. **Daemon Main Loop** (child, after connection):
   - Handle IPC commands (status, shutdown)
   - Monitor connection health (via OpenConnect callbacks)
   - Log connection events to systemd journal
   - On disconnect: Log error, exit 1

## Exit Codes

- `0`: Connection established successfully (or already connected)
- `1`: Authentication or network failure
- `2`: Configuration error (missing setup, invalid config)

## Output Examples

### Success

```
🔄 Connecting to vpn.example.com...
🔑 Generating OTP token...
🔐 Authenticating...
✅ VPN connected successfully!

Server: vpn.example.com (SSL)
Interface: tun0
IP Address: 10.0.1.42

Run `akon vpn status` to check connection details.
Run `akon vpn off` to disconnect.
```

### Already Connected

```
✅ VPN already connected

Server: vpn.example.com
Uptime: 2h 15m 38s

Run `akon vpn status` for details.
```

### Authentication Failure

```
🔄 Connecting to vpn.example.com...
🔑 Generating OTP token...
🔐 Authenticating...

❌ Authentication failed

Possible causes:
  - OTP secret is incorrect (run `akon setup` to reconfigure)
  - Server rejected credentials
  - Clock skew (check system time with NTP)

Check logs: journalctl -t akon -n 50

Exit code: 1
```

### Network Error

```
🔄 Connecting to vpn.example.com...

❌ Network error: Connection timed out

Possible causes:
  - Server is unreachable
  - Firewall blocking VPN protocol
  - Network interface down

Exit code: 1
```

### Configuration Error

```
❌ Error: VPN configuration not found

Run `akon setup` to configure credentials.

Exit code: 2
```

## State Changes

- Creates daemon process (PID written to `/tmp/akon-daemon.pid`)
- Creates Unix socket (`/tmp/akon-daemon.sock`)
- Updates connection state (stored in `/tmp/akon-state.json`)
- Creates VPN tunnel interface (e.g., `tun0`)
- Logs events:
  - `event=connection_attempt server=<url>`
  - `event=otp_generated` (no token value)
  - `event=authentication_success`
  - `event=connection_established interface=tun0 ip=<ip>`

## Security Requirements

- TOTP token MUST NOT appear in logs or stdout
- OTP secret MUST remain in `secrecy::Secret<T>` until passed to FFI
- Daemon process MUST drop privileges if running as root (future feature)
- PID file MUST have permissions 0644 (world-readable for status checks)
- Unix socket MUST have permissions 0700 (user-only access)

## Dependencies

- GNOME Keyring accessible (setup completed)
- libopenconnect library available (linked)
- Network connectivity to VPN server
- TUN/TAP kernel module loaded
- User permissions for network configuration (may require sudo in future)

## Process Model

```
┌─────────────┐
│ CLI Process │
│ (akon vpn on)│
└──────┬──────┘
       │ fork()
       ├────────────────────────────────────┐
       │                                    │
       ▼                                    ▼
┌──────────────┐                    ┌──────────────┐
│ Parent       │                    │ Child (Daemon)│
│              │                    │              │
│ 1. Listen on │                    │ 1. Connect   │
│    socket    │◀────signal─────────│    via FFI   │
│              │ ConnectionEstablished│            │
│ 2. Display   │                    │ 2. Enter     │
│    success   │                    │    main loop │
│              │                    │              │
│ 3. Exit 0    │                    │ 3. Handle IPC│
└──────────────┘                    │ 4. Monitor   │
                                     │    health    │
                                     └──────────────┘
```

## Test Scenarios

1. **Happy Path**: Valid config, keyring accessible, server reachable → Daemon spawned, exit 0
2. **Already Connected**: Daemon running → Display status, exit 0
3. **Auth Failure**: Invalid OTP secret → Display error, exit 1
4. **Network Timeout**: Server unreachable → Display error, exit 1
5. **Keyring Locked**: Locked keyring → Prompt unlock, retry once
6. **Missing Setup**: No config file → Display error, exit 2
7. **Foreground Mode**: `--foreground` flag → Run in foreground (Ctrl+C to stop)
8. **Connection Timeout**: No response in 30s → Kill daemon, exit 1

## Error Category Mapping (FR-009)

| OpenConnect Error | Error Category | Exit Code | User Message |
|-------------------|---------------|-----------|--------------|
| `OC_ERR_AUTHFAIL` | Authentication | 1 | "Authentication failed" + troubleshooting |
| `OC_ERR_NETWORK` | Network | 1 | "Network error: <details>" |
| `OC_ERR_PROTO` | Configuration | 2 | "Protocol error (check server config)" |
| Keyring locked | Configuration | 2 | "Keyring locked (unlock required)" |
| Missing config | Configuration | 2 | "Run `akon setup` first" |
