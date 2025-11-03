# Contract: VPN On Command (CLI Integration)

**Feature**: 002-refactor-openconnect-to
**Module**: `src/cli/vpn.rs::on_command()`
**Purpose**: CLI-based VPN connection establishment using OpenConnect process delegation

## Interface

### Function Signature

```rust
pub async fn on_command(config: VpnConfig) -> Result<(), AkonError>
```

### Parameters

- `config: VpnConfig` - VPN configuration from TOML file containing:
  - `server: String` - VPN server URL (e.g., "vpn.example.com")
  - `protocol: String` - Protocol identifier ("f5")
  - `username: String` - VPN username

### Returns

- `Ok(())` - Connection successfully established
- `Err(AkonError)` - Connection failed with specific error

### Error Cases

```rust
pub enum AkonError {
    VpnError(VpnError),
    ConfigError(String),
    KeyringError(String),
}

pub enum VpnError {
    ProcessSpawnError(String),      // OpenConnect not found or spawn failed
    AuthenticationError(String),     // Invalid credentials
    ConnectionTimeout(u64),          // Connection timeout (60s)
    TerminationError(String),        // Failed to terminate process
    OpenConnectError(String),        // Generic OpenConnect error
}
```

## Behavior

### Pre-conditions

1. VPN configuration file exists at `~/.config/akon/config.toml`
2. Credentials stored in GNOME Keyring:
   - Key: `akon:pin` (VPN password)
   - Key: `akon:otp_seed` (TOTP seed, optional)
3. OpenConnect 9.x CLI installed and available in PATH
4. User has root/sudo privileges (VPN requires elevated permissions)
5. No existing VPN connection active

### Post-conditions (Success)

1. OpenConnect process running as child process
2. TUN device configured with assigned IP address
3. VPN connection established (FR-001)
4. Connection state persisted (for `vpn status` command)
5. User sees success message with assigned IP
6. Exit code: 0

### Post-conditions (Failure)

1. No OpenConnect process running
2. No TUN device created
3. Credentials remain in keyring (unchanged)
4. User sees error message with diagnostic info
5. Exit code: 1

## Implementation Contract

### Step 1: Retrieve Credentials from Keyring

```rust
use akon_core::auth::{keyring, totp};

let pin = keyring::get_password("akon", "pin")
    .map_err(|e| AkonError::KeyringError(format!("Failed to retrieve PIN: {}", e)))?;

let otp_seed = keyring::get_password("akon", "otp_seed").ok();
let otp_token = if let Some(seed) = otp_seed {
    Some(totp::generate_totp(&seed)?)
} else {
    None
};

let credentials = Credentials { pin, otp: otp_token };
```

**Error Handling**:
- If PIN missing → `KeyringError("PIN not found in keyring. Run 'akon setup' first.")`
- If OTP seed present but TOTP generation fails → `VpnError::AuthenticationError`

### Step 2: Create CliConnector

```rust
use akon_core::vpn::CliConnector;

let mut connector = CliConnector::new(config)
    .map_err(AkonError::VpnError)?;
```

### Step 3: Initiate Connection

```rust
println!("Connecting to VPN server {}...", config.server);

connector.connect(credentials).await
    .map_err(AkonError::VpnError)?;
```

**User Feedback** (progressive):
```
Connecting to VPN server vpn.example.com...
Authenticating...
Establishing F5 session...
Configuring TUN device...
```

### Step 4: Monitor Connection Events

```rust
use akon_core::vpn::ConnectionEvent;
use tokio::time::{timeout, Duration};

let result = timeout(Duration::from_secs(60), async {
    while let Some(event) = connector.next_event().await {
        match event {
            ConnectionEvent::ProcessStarted { pid } => {
                tracing::info!(pid = %pid, "OpenConnect process started");
            }

            ConnectionEvent::Authenticating { message } => {
                println!("  {}", message);
            }

            ConnectionEvent::F5SessionEstablished { .. } => {
                println!("  ✓ F5 session established");
            }

            ConnectionEvent::TunConfigured { device, ip } => {
                println!("  ✓ TUN device {} configured", device);
                tracing::info!(device = %device, ip = %ip, "TUN configured");
            }

            ConnectionEvent::Connected { ip, device } => {
                println!("✓ Connected to VPN");
                println!("  IP Address: {}", ip);
                println!("  Device: {}", device);

                // Persist connection state
                save_connection_state(&ip, &device)?;

                return Ok(());
            }

            ConnectionEvent::Error { kind, raw_output } => {
                eprintln!("✗ Connection failed: {}", kind);
                eprintln!("\nDiagnostic output:");
                eprintln!("{}", raw_output);

                tracing::error!(
                    error = %kind,
                    raw_output = %raw_output,
                    "VPN connection failed"
                );

                return Err(AkonError::VpnError(kind));
            }

            ConnectionEvent::UnknownOutput { line } => {
                // Log but don't display to user (FR-015)
                tracing::debug!(line = %line, "Unparsed OpenConnect output");
            }

            _ => {
                // Other events logged but not displayed
                tracing::debug!(event = ?event, "Connection event");
            }
        }
    }

    Err(AkonError::VpnError(VpnError::ConnectionTimeout(60)))
}).await;

match result {
    Ok(Ok(())) => Ok(()),
    Ok(Err(e)) => Err(e),
    Err(_) => {
        eprintln!("✗ Connection timeout after 60 seconds");

        // Clean up failed connection
        let _ = connector.disconnect().await;

        Err(AkonError::VpnError(VpnError::ConnectionTimeout(60)))
    }
}
```

### Step 5: Persist Connection State

```rust
use std::fs;

fn save_connection_state(ip: &IpAddr, device: &str) -> Result<(), AkonError> {
    let state = ConnectionState {
        ip: *ip,
        device: device.to_string(),
        connected_at: chrono::Utc::now(),
    };

    let state_path = "/tmp/akon_vpn_state.json";
    let state_json = serde_json::to_string(&state)
        .map_err(|e| AkonError::ConfigError(e.to_string()))?;

    fs::write(state_path, state_json)
        .map_err(|e| AkonError::ConfigError(format!("Failed to save state: {}", e)))?;

    Ok(())
}
```

**State File Format** (`/tmp/akon_vpn_state.json`):
```json
{
  "ip": "10.0.1.100",
  "device": "tun0",
  "connected_at": "2025-11-03T10:30:00Z"
}
```

## Performance Requirements

- **Connection Establishment**: <30 seconds total (FR-005, Success Criteria SC-001)
- **Event Latency**: <500ms from OpenConnect output to user feedback (FR-003)
- **Memory Usage**: <50MB for connector + process monitoring

## Security Requirements

1. **Password Handling** (FR-002):
   - Retrieved from keyring only (never hardcoded)
   - Transmitted via stdin `--passwd-on-stdin` (not CLI args)
   - Not logged or printed to console

2. **Logging Exclusions** (FR-015):
   - PIN/password never logged
   - OTP tokens never logged
   - Session tokens redacted in logs (if present)
   - Only non-sensitive connection metadata logged

3. **Process Isolation**:
   - OpenConnect runs as separate process (child)
   - Process PID tracked for cleanup
   - No shared memory with VPN process

## Observability (FR-022)

All state transitions logged to systemd journal:

```rust
use tracing::{info, error, debug};

// Process started
info!(
    pid = %child_pid,
    server = %config.server,
    protocol = %config.protocol,
    "VPN process started"
);

// Authentication phase
debug!(
    server = %config.server,
    state = "Authenticating",
    "VPN state transition"
);

// Connection established
info!(
    ip = %assigned_ip,
    device = %tun_device,
    server = %config.server,
    state = "Connected",
    "VPN connection established"
);

// Error occurred
error!(
    server = %config.server,
    error = %error_kind,
    state = "Failed",
    "VPN connection failed"
);
```

**Journal Query Example**:
```bash
journalctl -u akon -o json-pretty | jq 'select(.state == "Connected")'
```

## Testing

### Unit Test Contract

```rust
#[tokio::test]
async fn test_on_command_success_flow() {
    // Mock dependencies
    let mock_keyring = MockKeyring::new()
        .expect_get_password("akon", "pin")
        .returning(|| Ok("test-pin".to_string()));

    let mock_connector = MockCliConnector::new()
        .expect_connect()
        .returning(|_| Ok(()))
        .expect_next_event()
        .returning(|| Some(ConnectionEvent::Connected {
            ip: "10.0.1.100".parse().unwrap(),
            device: "tun0".to_string(),
        }));

    // Execute
    let config = VpnConfig { /* test config */ };
    let result = on_command(config).await;

    // Assert
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_on_command_authentication_failure() {
    // Mock authentication failure
    let mock_connector = MockCliConnector::new()
        .expect_connect()
        .returning(|_| Ok(()))
        .expect_next_event()
        .returning(|| Some(ConnectionEvent::Error {
            kind: VpnError::AuthenticationError("Invalid PIN".to_string()),
            raw_output: "Failed to authenticate".to_string(),
        }));

    let config = VpnConfig { /* test config */ };
    let result = on_command(config).await;

    assert!(matches!(result, Err(AkonError::VpnError(
        VpnError::AuthenticationError(_)
    ))));
}

#[tokio::test]
async fn test_on_command_timeout() {
    // Mock connector that never emits Connected event
    let mock_connector = MockCliConnector::new()
        .expect_connect()
        .returning(|_| Ok(()))
        .expect_next_event()
        .returning(|| {
            // Simulate hanging connection
            std::thread::sleep(std::time::Duration::from_secs(2));
            None
        });

    let config = VpnConfig { /* test config */ };
    let result = on_command(config).await;

    assert!(matches!(result, Err(AkonError::VpnError(
        VpnError::ConnectionTimeout(_)
    ))));
}
```

### Integration Test Contract

```rust
#[tokio::test]
#[ignore] // Requires sudo and real OpenConnect
async fn test_real_vpn_connection() {
    // Setup: Ensure credentials in keyring
    setup_test_credentials();

    let config = VpnConfig {
        server: "test-vpn.example.com".to_string(),
        protocol: "f5".to_string(),
        username: "testuser".to_string(),
    };

    // Execute
    let result = on_command(config).await;

    // Assert
    assert!(result.is_ok());

    // Cleanup
    cleanup_vpn_connection().await;
}
```

## Edge Cases

1. **OpenConnect not installed**:
   - Error: `ProcessSpawnError("openconnect: command not found")`
   - User message: "OpenConnect CLI not found. Install with: sudo apt install openconnect"

2. **No keyring service available**:
   - Error: `KeyringError("Failed to connect to keyring service")`
   - User message: "Keyring service unavailable. Ensure GNOME Keyring is running."

3. **Insufficient permissions**:
   - Error: `ProcessSpawnError("Permission denied")`
   - User message: "VPN connection requires root privileges. Run with sudo."

4. **Concurrent connection attempt**:
   - Check existing state file before connecting
   - Error: "VPN already connected. Run 'akon vpn off' first."

5. **Network unavailable**:
   - OpenConnect stderr: "Cannot resolve hostname"
   - Error: `OpenConnectError("DNS resolution failed")`
   - User message includes raw output for diagnostics

## Dependencies

- `akon-core::vpn::CliConnector`
- `akon-core::vpn::ConnectionEvent`
- `akon-core::auth::{keyring, totp}`
- `akon-core::config::VpnConfig`
- `akon-core::error::{AkonError, VpnError}`
- `tokio::time::timeout`
- `tracing` (for observability)
- `serde_json` (for state persistence)

## Backward Compatibility

- Keyring keys unchanged (FR-010)
- TOML config format unchanged (FR-010)
- Command syntax unchanged: `akon vpn on`
- State file location may change (not part of public API)

## Future Enhancements

1. Connection retry with exponential backoff
2. Multiple VPN profile support
3. Auto-reconnect on disconnect
4. Connection quality metrics
5. Split tunneling configuration
