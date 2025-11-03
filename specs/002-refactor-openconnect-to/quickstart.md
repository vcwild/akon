# Quickstart: OpenConnect CLI Delegation Implementation

**Feature**: 002-refactor-openconnect-to
**Estimated Time**: 8-12 hours of development
**Prerequisites**: Rust 1.70+, OpenConnect 9.x installed, familiarity with async Rust

## Overview

This guide walks you through implementing the CLI-based OpenConnect integration from scratch using Test-Driven Development (TDD). Follow phases sequentially for best results.

## Setup (15 minutes)

### 1. Update Dependencies

Edit `akon-core/Cargo.toml`:

```toml
[dependencies]
tokio = { version = "1.35", features = ["process", "io-util", "time", "macros", "rt-multi-thread"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "1.0"
regex = "1.10"

# Keep existing dependencies
keyring = "..."
toml = "..."
serde = { version = "...", features = ["derive"] }

# REMOVE these FFI dependencies
# bindgen = "..."  # DELETE
# cc = "..."       # DELETE

[dev-dependencies]
criterion = "0.5"
tokio-test = "0.4"
```

### 2. Remove FFI Build Infrastructure

```bash
# Delete C wrapper files
rm akon-core/build.rs
rm akon-core/wrapper.h
rm akon-core/openconnect-internal.h
rm akon-core/progress_shim.c

# Delete FFI implementation
rm akon-core/src/vpn/openconnect.rs

# Delete FFI binding tests
rm tests/*_ffi_tests.rs  # Only FFI-specific tests
```

### 3. Create New Module Files

```bash
# VPN modules
touch akon-core/src/vpn/cli_connector.rs
touch akon-core/src/vpn/output_parser.rs
touch akon-core/src/vpn/connection_event.rs

# Test files
touch tests/unit/output_parser_tests.rs
touch tests/unit/cli_connector_tests.rs
touch tests/integration/credential_flow_tests.rs
```

### 4. Update Module Declarations

Edit `akon-core/src/vpn/mod.rs`:

```rust
// Remove FFI implementation
// mod openconnect;  // DELETE THIS LINE

// Add CLI implementation
mod cli_connector;
mod output_parser;
mod connection_event;

// Re-exports
pub use cli_connector::CliConnector;
pub use output_parser::OutputParser;
pub use connection_event::{ConnectionEvent, DisconnectReason};
```

## Phase 1: Connection Events (RED → GREEN → REFACTOR) [1 hour]

### Step 1: Write Failing Test

Create `tests/unit/connection_event_tests.rs`:

```rust
use akon_core::vpn::ConnectionEvent;

#[test]
fn test_connection_event_equality() {
    let event1 = ConnectionEvent::ProcessStarted { pid: 1234 };
    let event2 = ConnectionEvent::ProcessStarted { pid: 1234 };
    assert_eq!(event1, event2);
}

#[test]
fn test_connected_event_contains_ip() {
    use std::net::IpAddr;
    let ip: IpAddr = "10.0.1.100".parse().unwrap();
    let event = ConnectionEvent::Connected {
        ip,
        device: "tun0".to_string(),
    };

    match event {
        ConnectionEvent::Connected { ip, .. } => {
            assert_eq!(ip.to_string(), "10.0.1.100");
        }
        _ => panic!("Wrong event type"),
    }
}
```

Run: `cargo test connection_event` → **FAILS** (module doesn't exist)

### Step 2: Minimal Implementation (GREEN)

Create `akon-core/src/vpn/connection_event.rs`:

```rust
use std::net::IpAddr;
use crate::error::VpnError;

/// Events emitted during OpenConnect CLI connection lifecycle
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    ProcessStarted { pid: u32 },
    Authenticating { message: String },
    F5SessionEstablished { session_token: Option<String> },
    TunConfigured { device: String, ip: IpAddr },
    Connected { ip: IpAddr, device: String },
    Disconnected { reason: DisconnectReason },
    Error { kind: VpnError, raw_output: String },
    UnknownOutput { line: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    UserRequested,
    ServerDisconnect,
    ProcessTerminated,
    Timeout,
}
```

Run: `cargo test connection_event` → **PASSES**

### Step 3: Refactor (if needed)

Add documentation, derive macros. Tests should stay green.

## Phase 2: Output Parser (TDD) [2 hours]

### Step 1: Write Failing Tests

Create `tests/unit/output_parser_tests.rs`:

```rust
use akon_core::vpn::{OutputParser, ConnectionEvent};
use std::net::IpAddr;

#[test]
fn test_parse_tun_configured() {
    let parser = OutputParser::new();
    let line = "Connected tun0 as 10.0.1.100";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::TunConfigured { device, ip } => {
            assert_eq!(device, "tun0");
            assert_eq!(ip, "10.0.1.100".parse::<IpAddr>().unwrap());
        }
        _ => panic!("Wrong event: {:?}", event),
    }
}

#[test]
fn test_parse_established_connection() {
    let parser = OutputParser::new();
    let line = "Established connection";
    let event = parser.parse_line(line);

    assert!(matches!(event, ConnectionEvent::Authenticating { .. }));
}

#[test]
fn test_parse_authentication_failure() {
    let parser = OutputParser::new();
    let line = "Failed to authenticate";
    let event = parser.parse_error(line);

    match event {
        ConnectionEvent::Error { kind, .. } => {
            // Verify it's an authentication error
        }
        _ => panic!("Expected Error event"),
    }
}

#[test]
fn test_parse_unknown_output() {
    let parser = OutputParser::new();
    let line = "Some random output from OpenConnect";
    let event = parser.parse_line(line);

    match event {
        ConnectionEvent::UnknownOutput { line: l } => {
            assert_eq!(l, line);
        }
        _ => panic!("Expected UnknownOutput"),
    }
}
```

Run: `cargo test output_parser` → **FAILS** (module doesn't exist)

### Step 2: Minimal Implementation (GREEN)

Create `akon-core/src/vpn/output_parser.rs`:

```rust
use crate::vpn::ConnectionEvent;
use crate::error::VpnError;
use regex::Regex;
use std::net::IpAddr;

pub struct OutputParser {
    tun_configured_pattern: Regex,
    established_pattern: Regex,
    auth_failed_pattern: Regex,
}

impl OutputParser {
    pub fn new() -> Self {
        Self {
            tun_configured_pattern: Regex::new(r"Connected (\w+) as ([\d.]+)").unwrap(),
            established_pattern: Regex::new(r"Established connection").unwrap(),
            auth_failed_pattern: Regex::new(r"Failed to authenticate").unwrap(),
        }
    }

    pub fn parse_line(&self, line: &str) -> ConnectionEvent {
        // Try TUN configured pattern
        if let Some(caps) = self.tun_configured_pattern.captures(line) {
            let device = caps.get(1).unwrap().as_str().to_string();
            let ip: IpAddr = caps.get(2).unwrap().as_str().parse().unwrap();
            return ConnectionEvent::TunConfigured { device, ip };
        }

        // Try established connection
        if self.established_pattern.is_match(line) {
            return ConnectionEvent::Authenticating {
                message: "Connection established".to_string(),
            };
        }

        // Fallback to unknown
        ConnectionEvent::UnknownOutput {
            line: line.to_string(),
        }
    }

    pub fn parse_error(&self, line: &str) -> ConnectionEvent {
        if self.auth_failed_pattern.is_match(line) {
            return ConnectionEvent::Error {
                kind: VpnError::AuthenticationError(line.to_string()),
                raw_output: line.to_string(),
            };
        }

        ConnectionEvent::UnknownOutput {
            line: line.to_string(),
        }
    }
}
```

Run: `cargo test output_parser` → **PASSES**

### Step 3: Refactor

Extract pattern initialization to helper methods, add more patterns.

## Phase 3: CLI Connector Skeleton (TDD) [3 hours]

### Step 1: Write High-Level Test

Create `tests/unit/cli_connector_tests.rs`:

```rust
use akon_core::vpn::{CliConnector, ConnectionEvent};
use akon_core::config::VpnConfig;
use akon_core::auth::Credentials;

#[tokio::test]
async fn test_connector_initial_state_is_idle() {
    let config = VpnConfig {
        server: "vpn.example.com".to_string(),
        protocol: "f5".to_string(),
        username: "testuser".to_string(),
    };

    let connector = CliConnector::new(config).unwrap();
    assert!(!connector.is_connected());
}

#[tokio::test]
async fn test_connect_spawns_process() {
    // This test requires mocking - see Phase 4
}
```

Run: `cargo test cli_connector` → **FAILS** (module doesn't exist)

### Step 2: Minimal Implementation (GREEN)

Create `akon-core/src/vpn/cli_connector.rs`:

```rust
use crate::vpn::{ConnectionEvent, OutputParser};
use crate::config::VpnConfig;
use crate::auth::Credentials;
use crate::error::VpnError;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
enum ConnectionState {
    Idle,
    Connecting,
    Authenticating,
    Established { ip: std::net::IpAddr, device: String },
    Disconnecting,
    Failed { error: String },
}

pub struct CliConnector {
    state: Arc<Mutex<ConnectionState>>,
    config: VpnConfig,
    parser: Arc<OutputParser>,
}

impl CliConnector {
    pub fn new(config: VpnConfig) -> Result<Self, VpnError> {
        Ok(Self {
            state: Arc::new(Mutex::new(ConnectionState::Idle)),
            config,
            parser: Arc::new(OutputParser::new()),
        })
    }

    pub fn is_connected(&self) -> bool {
        // For now, always false
        false
    }
}
```

Run: `cargo test cli_connector` → **PASSES**

### Step 3: Add Connection Method (RED → GREEN)

Write test first:

```rust
#[tokio::test]
async fn test_connect_changes_state() {
    let config = VpnConfig { /* ... */ };
    let mut connector = CliConnector::new(config).unwrap();

    let creds = Credentials::mock(); // Need to add mock helper

    // This will fail until we implement connect()
    connector.connect(creds).await.unwrap();
}
```

Then implement `connect()` method (see contracts for full signature).

## Phase 4: Process Management Integration [3 hours]

### Step 1: Implement Process Spawning

Add to `cli_connector.rs`:

```rust
use tokio::process::{Command, Child};
use tokio::io::{AsyncWriteExt, BufReader, AsyncBufReadExt};

impl CliConnector {
    async fn spawn_process(
        &mut self,
        credentials: &Credentials,
    ) -> Result<Child, VpnError> {
        let mut cmd = Command::new("openconnect");
        cmd.args(&[
            "--protocol", &self.config.protocol,
            "--user", &self.config.username,
            "--passwd-on-stdin",
            &self.config.server,
        ]);

        cmd.stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()
            .map_err(|e| VpnError::ProcessSpawnError(e.to_string()))?;

        // Send password to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(credentials.pin.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            drop(stdin); // Close stdin
        }

        Ok(child)
    }
}
```

### Step 2: Implement Output Monitoring

Add background tasks:

```rust
use tokio::sync::mpsc;

impl CliConnector {
    async fn monitor_stdout(
        stdout: tokio::process::ChildStdout,
        parser: Arc<OutputParser>,
        event_sender: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let mut reader = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            let event = parser.parse_line(&line);
            let _ = event_sender.send(event);

            // Log with tracing
            tracing::debug!(line = %line, "OpenConnect stdout");
        }
    }

    async fn monitor_stderr(
        stderr: tokio::process::ChildStderr,
        parser: Arc<OutputParser>,
        event_sender: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let mut reader = BufReader::new(stderr).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            let event = parser.parse_error(&line);
            let _ = event_sender.send(event);

            // Log errors
            tracing::warn!(line = %line, "OpenConnect stderr");
        }
    }
}
```

### Step 3: Wire Up Connect Method

```rust
pub async fn connect(
    &mut self,
    credentials: Credentials,
) -> Result<(), VpnError> {
    // Update state
    *self.state.lock().await = ConnectionState::Connecting;

    // Spawn process
    let mut child = self.spawn_process(&credentials).await?;

    // Create event channel
    let (tx, rx) = mpsc::unbounded_channel();
    self.event_receiver = Some(rx);

    // Take stdout/stderr
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Spawn monitors
    let parser = self.parser.clone();
    tokio::spawn(Self::monitor_stdout(stdout, parser.clone(), tx.clone()));
    tokio::spawn(Self::monitor_stderr(stderr, parser, tx));

    // Store child process
    *self.child_process.lock().await = Some(child);

    Ok(())
}
```

## Phase 5: Graceful Termination [1 hour]

### Step 1: Write Test

```rust
#[tokio::test]
async fn test_disconnect_sends_sigterm() {
    // Setup connected state
    let mut connector = setup_connected_connector().await;

    connector.disconnect().await.unwrap();

    // Verify process exited
    assert!(!connector.is_connected());
}
```

### Step 2: Implement Disconnect

```rust
pub async fn disconnect(&mut self) -> Result<(), VpnError> {
    *self.state.lock().await = ConnectionState::Disconnecting;

    let mut child_guard = self.child_process.lock().await;
    if let Some(mut child) = child_guard.take() {
        // Send SIGTERM
        child.kill().await
            .map_err(|e| VpnError::TerminationError(e.to_string()))?;

        // Wait with timeout
        match tokio::time::timeout(
            Duration::from_secs(5),
            child.wait()
        ).await {
            Ok(_) => {
                tracing::info!("Process exited gracefully");
            }
            Err(_) => {
                // Force kill
                self.force_kill().await?;
            }
        }
    }

    *self.state.lock().await = ConnectionState::Idle;
    Ok(())
}

pub async fn force_kill(&mut self) -> Result<(), VpnError> {
    // Use nix crate for SIGKILL
    // Implementation details in contracts/vpn-off-command.md
    Ok(())
}
```

## Phase 6: CLI Integration [1 hour]

Update `src/cli/vpn.rs`:

```rust
use akon_core::vpn::CliConnector;

pub async fn on_command(config: VpnConfig) -> Result<(), AkonError> {
    // Get credentials from keyring
    let credentials = get_credentials_from_keyring(&config)?;

    // Create connector
    let mut connector = CliConnector::new(config)?;

    // Connect
    println!("Connecting to VPN...");
    connector.connect(credentials).await?;

    // Monitor events
    while let Some(event) = connector.next_event().await {
        match event {
            ConnectionEvent::Connected { ip, .. } => {
                println!("✓ Connected with IP: {}", ip);
                break;
            }
            ConnectionEvent::Error { kind, raw_output } => {
                eprintln!("✗ Connection failed: {}", kind);
                eprintln!("  Raw output: {}", raw_output);
                return Err(kind.into());
            }
            _ => {
                // Progress updates
                println!("  {}", event_to_string(&event));
            }
        }
    }

    Ok(())
}
```

## Phase 7: Testing & Validation [1-2 hours]

### Run Test Suite

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test integration

# Check coverage (requires tarpaulin)
cargo tarpaulin --out Html
```

### Manual Testing

```bash
# Build
cargo build --release

# Test VPN on
sudo ./target/release/akon vpn on

# Test VPN off
sudo ./target/release/akon vpn off

# Test status
./target/release/akon vpn status
```

### Performance Benchmarks

```bash
cargo bench
```

## Troubleshooting

### Issue: "openconnect: command not found"

**Solution**: Install OpenConnect 9.x:
```bash
sudo apt install openconnect  # Ubuntu/Debian
sudo dnf install openconnect  # Fedora
```

### Issue: "Permission denied" when spawning process

**Solution**: Run with sudo (VPN connections require root):
```bash
sudo akon vpn on
```

### Issue: Tests fail with "Resource temporarily unavailable"

**Solution**: Mock `Command::spawn()` in tests using `mockall` crate.

### Issue: Connection hangs during authentication

**Solution**: Check OpenConnect output manually:
```bash
echo "your-pin" | sudo openconnect --protocol=f5 --passwd-on-stdin vpn.example.com
```

## Next Steps

1. Review generated contracts in `contracts/` directory
2. Implement remaining functional requirements (FR-005 to FR-024)
3. Add comprehensive error handling
4. Implement observability (tracing integration)
5. Run full test suite with coverage check
6. Performance profiling and optimization
7. Documentation updates

## Useful Resources

- [Tokio Process Documentation](https://docs.rs/tokio/latest/tokio/process/)
- [OpenConnect Manual](https://www.infradead.org/openconnect/manual.html)
- [Rust Async Book](https://rust-lang.github.io/async-book/)
- [TDD in Rust](https://rust-unofficial.github.io/patterns/testing/tdd.html)
- Project data model: `data-model.md`
- Research decisions: `research.md`
