# Data Model

**Feature**: OTP-Integrated VPN CLI with Secure Credential Management
**Date**: 2025-10-08

## Overview

This document defines the core data structures for the Akon VPN CLI, extracted from functional requirements. All entities follow Rust best practices with `serde` derives for serialization and `secrecy` wrappers for sensitive data.

---

## 1. VPN Configuration

**Source**: FR-003, Key Entities section

**Description**: Non-sensitive connection parameters stored in `~/.config/akon/config.toml`.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VpnConfig {
    /// VPN server URL (e.g., "https://vpn.example.com")
    pub server: String,

    /// Username for authentication
    pub username: String,

    /// VPN protocol: SSL or NC (AnyConnect)
    pub protocol: VpnProtocol,

    /// Optional custom port (defaults to protocol standard)
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VpnProtocol {
    /// SSL VPN
    Ssl,

    /// Network Connect (AnyConnect)
    Nc,
}

impl VpnConfig {
    /// Validate configuration fields
    pub fn validate(&self) -> Result<(), ConfigError> {
        // URL validation
        url::Url::parse(&self.server)
            .map_err(|_| ConfigError::InvalidServerUrl(self.server.clone()))?;

        // Username validation
        if self.username.trim().is_empty() {
            return Err(ConfigError::EmptyUsername);
        }

        // Port range validation
        if let Some(port) = self.port {
            if port == 0 {
                return Err(ConfigError::InvalidPort(port));
            }
        }

        Ok(())
    }
}
```

**Relationships**:

- References `OtpSecret` (stored separately in keyring)
- Used by `VpnConnection` to establish tunnel

**Lifecycle**:

- Created during `akon setup` command
- Loaded on every VPN operation
- Updated via future `akon config update` command (out of scope for this iteration)

---

## 2. OTP Secret

**Source**: FR-002, FR-005, FR-007, Key Entities section

**Description**: Base32-encoded TOTP seed stored exclusively in GNOME Keyring. In-memory representation wrapped in `secrecy::Secret<T>`.

```rust
use secrecy::{Secret, ExposeSecret, Zeroize};
use serde::{Deserialize, Serialize};

/// Newtype wrapper for OTP secret with validation
#[derive(Clone)]
pub struct OtpSecret(Secret<String>);

impl OtpSecret {
    /// Create OTP secret from Base32-encoded string
    pub fn from_base32(input: String) -> Result<Self, OtpError> {
        // Validate Base32 format (A-Z, 2-7, optional padding)
        if !Self::is_valid_base32(&input) {
            return Err(OtpError::InvalidBase32Format);
        }

        // Validate length (typically 16-32 characters)
        if input.len() < 16 || input.len() > 52 {
            return Err(OtpError::InvalidSecretLength(input.len()));
        }

        Ok(Self(Secret::new(input)))
    }

    /// Expose secret for TOTP generation (use sparingly)
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    fn is_valid_base32(s: &str) -> bool {
        s.chars().all(|c| {
            matches!(c, 'A'..='Z' | '2'..='7' | '=')
        })
    }
}

// Prevent accidental logging
impl std::fmt::Debug for OtpSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OtpSecret(***REDACTED***)")
    }
}
```

**Storage**:

- **Location**: GNOME Keyring (Secret Service API)
- **Service Name**: `akon-vpn-otp`
- **Attributes**: `{ "application": "akon", "type": "otp", "username": "<user>" }`
- **Encryption**: Handled by GNOME Keyring (session key encryption)

**Lifecycle**:

- Created during `akon setup` (user input or file import)
- Retrieved for each TOTP generation
- Never transmitted over network or written to disk
- Deleted on explicit `akon setup --reset` (future feature)

---

## 3. TOTP Token

**Source**: FR-004, FR-011

**Description**: Time-based one-time password generated from OTP secret. Short-lived (30-second validity).

```rust
use secrecy::Secret;
use std::time::{SystemTime, UNIX_EPOCH};

/// TOTP token with metadata
pub struct TotpToken {
    /// The actual token value (6-8 digits)
    value: Secret<String>,

    /// Unix timestamp when token was generated
    generated_at: u64,

    /// Time step in seconds (typically 30)
    time_step: u64,
}

impl TotpToken {
    /// Generate TOTP from secret
    pub fn generate(secret: &OtpSecret, time_step: u64) -> Result<Self, OtpError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs();

        let secret_bytes = base32::decode(
            base32::Alphabet::RFC4648 { padding: false },
            secret.expose()
        ).ok_or(OtpError::Base32DecodeFailed)?;

        let token_value = totp_lite::totp::<totp_lite::Sha1>(
            &secret_bytes,
            time_step,
            now
        );

        let formatted = format!("{:06}", token_value);  // Zero-pad to 6 digits

        Ok(Self {
            value: Secret::new(formatted),
            generated_at: now,
            time_step,
        })
    }

    /// Get token value (for OpenConnect FFI or stdout)
    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    /// Check if token is still valid
    pub fn is_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let elapsed = now - self.generated_at;
        elapsed < self.time_step
    }

    /// Seconds remaining before expiration
    pub fn remaining_seconds(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let elapsed = now - self.generated_at;
        self.time_step.saturating_sub(elapsed)
    }
}

// Prevent accidental logging
impl std::fmt::Debug for TotpToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpToken")
            .field("value", &"***REDACTED***")
            .field("generated_at", &self.generated_at)
            .field("time_step", &self.time_step)
            .finish()
    }
}
```

**Lifecycle**:

- Generated on-demand for VPN connection or `get-password` command
- Exists in memory only (never persisted)
- Automatically expires after `time_step` seconds
- Zeroized on drop (via `secrecy` crate)

---

## 4. Connection State

**Source**: FR-008, FR-012, Key Entities section

**Description**: Current VPN connection status with metadata. Shared between CLI process and daemon via IPC.

```rust
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionState {
    /// Current connection status
    pub status: ConnectionStatus,

    /// VPN server endpoint (from config)
    pub server: String,

    /// Unix timestamp when connection was established (if connected)
    pub connected_at: Option<u64>,

    /// Last error message (if status is Error)
    pub last_error: Option<String>,

    /// Process ID of daemon (if running)
    pub daemon_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    /// No active connection
    Disconnected,

    /// Connection attempt in progress
    Connecting,

    /// Connected and authenticated
    Connected,

    /// Connection failed or dropped
    Error,
}

impl ConnectionState {
    /// Create new disconnected state
    pub fn disconnected() -> Self {
        Self {
            status: ConnectionStatus::Disconnected,
            server: String::new(),
            connected_at: None,
            last_error: None,
            daemon_pid: None,
        }
    }

    /// Get uptime in seconds (if connected)
    pub fn uptime(&self) -> Option<u64> {
        match (self.status, self.connected_at) {
            (ConnectionStatus::Connected, Some(start)) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                Some(now - start)
            }
            _ => None,
        }
    }

    /// Format uptime as human-readable string
    pub fn uptime_string(&self) -> String {
        match self.uptime() {
            Some(secs) => {
                let hours = secs / 3600;
                let minutes = (secs % 3600) / 60;
                let seconds = secs % 60;
                format!("{}h {}m {}s", hours, minutes, seconds)
            }
            None => "N/A".to_string(),
        }
    }
}
```

**Concurrency**:

- Wrapped in `Arc<Mutex<ConnectionState>>` for shared access
- Updated by daemon process, read by CLI commands
- Synchronized via Unix socket IPC (serialized as JSON)

**Lifecycle**:

- Created on daemon startup (status: Connecting)
- Updated on connection success/failure
- Persisted to state file (`/tmp/akon-state.json`) for crash recovery
- Cleared on clean shutdown

---

## 5. Keyring Entry

**Source**: FR-002, FR-016, Key Entities section

**Description**: Metadata for secrets stored in GNOME Keyring.

```rust
/// Keyring entry identifier
pub struct KeyringEntry {
    /// Service name (e.g., "akon-vpn-otp")
    pub service: String,

    /// Username/account identifier
    pub username: String,

    /// Search attributes for Secret Service API
    pub attributes: Vec<(String, String)>,
}

impl KeyringEntry {
    /// Create entry for OTP secret
    pub fn otp_secret(username: &str) -> Self {
        Self {
            service: "akon-vpn-otp".to_string(),
            username: username.to_string(),
            attributes: vec![
                ("application".to_string(), "akon".to_string()),
                ("type".to_string(), "otp".to_string()),
                ("username".to_string(), username.to_string()),
            ],
        }
    }

    /// Create entry for VPN password (if needed)
    pub fn vpn_password(username: &str) -> Self {
        Self {
            service: "akon-vpn-password".to_string(),
            username: username.to_string(),
            attributes: vec![
                ("application".to_string(), "akon".to_string()),
                ("type".to_string(), "password".to_string()),
                ("username".to_string(), username.to_string()),
            ],
        }
    }
}
```

**Operations**:

- `store(entry, secret)`: Create or update keyring entry
- `retrieve(entry) -> Secret<String>`: Fetch secret by attributes
- `delete(entry)`: Remove from keyring
- `exists(entry) -> bool`: Check if entry exists

---

## 6. Daemon IPC Message

**Source**: FR-012 (daemon communication for `vpn on/off/status`)

**Description**: Messages exchanged between CLI and daemon via Unix socket.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Request daemon to shutdown gracefully
    Shutdown,

    /// Query current connection status
    StatusRequest,

    /// Response to status request
    StatusResponse(ConnectionState),

    /// Signal connection established successfully
    ConnectionEstablished {
        server: String,
        timestamp: u64,
    },

    /// Signal connection failed
    ConnectionFailed {
        error: String,
    },

    /// Heartbeat (daemon alive check)
    Ping,

    /// Heartbeat response
    Pong,
}

impl IpcMessage {
    /// Serialize to JSON for socket transmission
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from JSON
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}
```

**Communication Pattern**:

1. CLI connects to Unix socket (`/tmp/akon-daemon.sock`)
2. CLI sends request message (e.g., `StatusRequest`)
3. Daemon processes request and sends response
4. CLI disconnects after receiving response

---

## Entity Relationships

```
┌─────────────────┐
│  VpnConfig      │
│  (TOML file)    │
└────────┬────────┘
         │
         │ references
         │
         ▼
┌─────────────────┐      generates      ┌──────────────┐
│  OtpSecret      │─────────────────────▶│ TotpToken    │
│  (Keyring)      │                      │ (memory)     │
└─────────────────┘                      └──────┬───────┘
                                                │
                                                │ used by
                                                │
                                                ▼
┌─────────────────┐      updates        ┌──────────────┐
│  Daemon Process │─────────────────────▶│ ConnectionState│
│                 │                      │ (shared)     │
└────────┬────────┘                      └──────▲───────┘
         │                                      │
         │ IPC via                              │ queries
         │ Unix socket                          │
         ▼                                      │
┌─────────────────┐                      ┌──────┴───────┐
│  IpcMessage     │──────────────────────▶│  CLI Process │
│  (JSON)         │      StatusResponse   │              │
└─────────────────┘                       └──────────────┘
```

---

## Data Validation Rules

| Entity | Field | Validation Rule | Error Type |
|--------|-------|----------------|------------|
| VpnConfig | `server` | Valid URL format | `ConfigError::InvalidServerUrl` |
| VpnConfig | `username` | Non-empty, trimmed | `ConfigError::EmptyUsername` |
| VpnConfig | `port` | 1-65535 or None | `ConfigError::InvalidPort` |
| OtpSecret | `value` | Base32 alphabet only | `OtpError::InvalidBase32Format` |
| OtpSecret | `value` | 16-52 characters | `OtpError::InvalidSecretLength` |
| TotpToken | generation | System time available | `OtpError::SystemTimeError` |
| TotpToken | validity | Within time step | N/A (method check) |

---

## State Transitions

### Connection State Machine

```
┌──────────────┐
│ Disconnected │
└──────┬───────┘
       │ vpn on
       ▼
┌──────────────┐
│  Connecting  │
└──────┬───────┘
       │
       ├─ success ──▶ ┌──────────┐
       │              │ Connected │
       │              └─────┬─────┘
       │                    │ vpn off
       │                    │ or error
       │                    ▼
       └─ failure ───▶ ┌──────────┐
                       │   Error  │
                       └─────┬─────┘
                             │ vpn on (retry)
                             ▼
                       ┌──────────────┐
                       │ Disconnected │
                       └──────────────┘
```

---

## Security Properties

| Entity | Sensitivity | Protection Mechanism |
|--------|-------------|----------------------|
| `OtpSecret` | High | `secrecy::Secret<T>`, GNOME Keyring encryption |
| `TotpToken` | High | `secrecy::Secret<T>`, auto-zeroize on drop |
| `VpnConfig` | Low | File permissions (0600), no secrets |
| `ConnectionState` | Low | Transient, no secrets, world-readable |
| `IpcMessage` | Low | Unix socket permissions (user-only) |

**Invariants**:

- OTP secret never appears in Debug output (custom impl)
- TOTP token never logged or serialized to disk
- Configuration file contains zero secrets
- Keyring operations always check for locked state before access

---

## Summary

All data models are finalized with:

✅ Rust type definitions with `serde` derives
✅ Validation rules mapped to error types
✅ Lifecycle documentation (creation, usage, deletion)
✅ Security properties (sensitivity classification, protection mechanisms)
✅ State transitions for `ConnectionState`
✅ Entity relationships diagram

**Next**: Generate API contracts (CLI commands) in Phase 1.
