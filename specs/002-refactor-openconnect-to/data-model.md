# Data Model: OpenConnect CLI Delegation

**Feature**: 002-refactor-openconnect-to
**Purpose**: Define core entities, their relationships, and state machines for CLI-based VPN connection management

## Overview

The data model centers on three key entities:
1. **ConnectionEvent**: State machine events representing VPN lifecycle
2. **CliConnector**: Process manager spawning and monitoring OpenConnect
3. **OutputParser**: Pattern matcher extracting events from CLI output

## Entity Definitions

### 1. ConnectionEvent

**Purpose**: Type-safe state machine events for VPN connection lifecycle

```rust
/// Events emitted during OpenConnect CLI connection lifecycle
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// OpenConnect process started successfully
    ProcessStarted {
        pid: u32,
    },

    /// Authentication phase in progress
    Authenticating {
        message: String,
    },

    /// F5 session manager connection established
    F5SessionEstablished {
        session_token: Option<String>, // May be redacted for security
    },

    /// TUN device configured with assigned IP
    TunConfigured {
        device: String,  // e.g., "tun0"
        ip: std::net::IpAddr,
    },

    /// Full VPN connection established
    Connected {
        ip: std::net::IpAddr,
        device: String,
    },

    /// Connection disconnected normally
    Disconnected {
        reason: DisconnectReason,
    },

    /// Error occurred during connection
    Error {
        kind: VpnError,
        raw_output: String,
    },

    /// Unparsed output line (fallback)
    UnknownOutput {
        line: String,
    },
}

/// Reasons for disconnection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    UserRequested,
    ServerDisconnect,
    ProcessTerminated,
    Timeout,
}
```

**State Transitions**:
```
[Start] → ProcessStarted → Authenticating → F5SessionEstablished → TunConfigured → Connected
                                ↓                    ↓                    ↓              ↓
                              Error              Error              Error        Disconnected
```

**Relationships**:
- Produced by: `OutputParser::parse_line()`
- Consumed by: `CliConnector` (internal state tracking), `cli::vpn` commands (user feedback)

### 2. CliConnector

**Purpose**: Manages OpenConnect CLI process lifecycle from spawn to termination

```rust
/// CLI-based OpenConnect connection manager
pub struct CliConnector {
    /// Current connection state
    state: Arc<Mutex<ConnectionState>>,

    /// Optional handle to running OpenConnect process
    child_process: Arc<Mutex<Option<tokio::process::Child>>>,

    /// Channel for receiving connection events
    event_receiver: tokio::sync::mpsc::UnboundedReceiver<ConnectionEvent>,

    /// Parser for OpenConnect output
    parser: Arc<OutputParser>,

    /// Configuration (server URL, protocol)
    config: VpnConfig,
}

/// Internal connection state
#[derive(Debug, Clone, PartialEq)]
enum ConnectionState {
    Idle,
    Connecting,
    Authenticating,
    Established { ip: std::net::IpAddr, device: String },
    Disconnecting,
    Failed { error: VpnError },
}
```

**Key Methods**:

```rust
impl CliConnector {
    /// Create new connector with configuration
    pub fn new(config: VpnConfig) -> Result<Self, AkonError>;

    /// Start VPN connection asynchronously
    ///
    /// Spawns OpenConnect process, passes credentials via stdin,
    /// monitors stdout/stderr in background tasks
    pub async fn connect(
        &mut self,
        credentials: Credentials,
    ) -> Result<(), VpnError>;

    /// Gracefully disconnect VPN (SIGTERM with 5s timeout)
    pub async fn disconnect(&mut self) -> Result<(), VpnError>;

    /// Force-kill VPN connection (SIGKILL)
    pub async fn force_kill(&mut self) -> Result<(), VpnError>;

    /// Get current connection state
    pub fn state(&self) -> ConnectionState;

    /// Wait for next connection event (non-blocking)
    pub async fn next_event(&mut self) -> Option<ConnectionEvent>;

    /// Check if connection is active
    pub fn is_connected(&self) -> bool;
}

// Private helper methods
impl CliConnector {
    /// Spawn OpenConnect process with arguments
    async fn spawn_process(
        &mut self,
        credentials: &Credentials,
    ) -> Result<tokio::process::Child, VpnError>;

    /// Monitor stdout in background task
    async fn monitor_stdout(
        stdout: tokio::process::ChildStdout,
        parser: Arc<OutputParser>,
        event_sender: tokio::sync::mpsc::UnboundedSender<ConnectionEvent>,
    );

    /// Monitor stderr in background task
    async fn monitor_stderr(
        stderr: tokio::process::ChildStderr,
        parser: Arc<OutputParser>,
        event_sender: tokio::sync::mpsc::UnboundedSender<ConnectionEvent>,
    );

    /// Write password to stdin and close
    async fn send_password(
        stdin: &mut tokio::process::ChildStdin,
        password: &SecureString,
    ) -> Result<(), VpnError>;
}
```

**Relationships**:
- Uses: `OutputParser` (parses CLI output)
- Produces: `ConnectionEvent` stream
- Consumes: `VpnConfig` (server, protocol), `Credentials` (keyring data)
- Interacts with: `tokio::process::Child` (OpenConnect process)

### 3. OutputParser

**Purpose**: Parse OpenConnect CLI stdout/stderr into typed `ConnectionEvent`s

```rust
/// Pattern-based parser for OpenConnect 9.x output
pub struct OutputParser {
    /// Patterns for F5 protocol (extensible to other protocols)
    patterns: Vec<OutputPattern>,
}

/// Pattern matching rule for output lines
struct OutputPattern {
    regex: regex::Regex,
    event_builder: fn(regex::Captures) -> ConnectionEvent,
}
```

**Key Methods**:

```rust
impl OutputParser {
    /// Create parser with F5 protocol patterns
    pub fn new() -> Self;

    /// Parse single output line into connection event
    ///
    /// Returns appropriate ConnectionEvent or UnknownOutput fallback
    pub fn parse_line(&self, line: &str) -> ConnectionEvent;

    /// Parse error line from stderr
    pub fn parse_error(&self, line: &str) -> ConnectionEvent;
}

// Private pattern matching
impl OutputParser {
    /// Initialize F5 output patterns
    fn init_f5_patterns() -> Vec<OutputPattern>;

    /// Extract IP address from line
    fn extract_ip(line: &str) -> Option<std::net::IpAddr>;
}
```

**Pattern Examples** (internal):
```rust
// "Connected tun0 as 10.0.1.100" → TunConfigured
regex!(r"Connected (\w+) as ([\d.]+)");

// "Established connection" → Connected
regex!(r"Established connection");

// "Failed to authenticate" → Error
regex!(r"Failed to authenticate");

// "Got CONNECT response: HTTP/1.1 200 OK" → Authenticating
regex!(r"Got CONNECT response: HTTP/[\d.]+ 200 OK");
```

**Relationships**:
- Used by: `CliConnector` (stdout/stderr monitoring)
- Produces: `ConnectionEvent` instances
- No external dependencies (pure parsing logic)

## Entity Relationships

```
┌─────────────────┐
│  VpnConfig      │ (Input)
│  Credentials    │
└────────┬────────┘
         │
         ↓
┌─────────────────────────────────┐
│  CliConnector                   │
│  - Spawns OpenConnect process   │
│  - Monitors stdout/stderr       │────┐
│  - Manages state transitions    │    │
└─────────────────────────────────┘    │
         │                              │
         │ uses                         │ output lines
         ↓                              ↓
┌─────────────────────────────────┐    │
│  OutputParser                   │←───┘
│  - Pattern matching             │
│  - Event extraction             │
└─────────────────────────────────┘
         │
         │ produces
         ↓
┌─────────────────────────────────┐
│  ConnectionEvent                │ (Output)
│  - State machine events         │
└─────────────────────────────────┘
         │
         │ consumed by
         ↓
┌─────────────────────────────────┐
│  cli::vpn commands              │
│  - User feedback                │
│  - Exit codes                   │
└─────────────────────────────────┘
```

## Data Flow

### 1. Connection Establishment Flow

```
User Command
    ↓
cli::vpn::on()
    ↓
CliConnector::connect(credentials)
    ↓
spawn_process() → tokio::process::Command
    ↓
send_password() → stdin.write_all(password)
    ↓
┌──────────────────────────────────────┐
│  Background Tasks (tokio::spawn)     │
│                                      │
│  monitor_stdout() ───→ OutputParser │
│       ↓                     ↓        │
│  BufReader::lines()   parse_line()   │
│       ↓                     ↓        │
│  event_sender ←─────  ConnectionEvent│
└──────────────────────────────────────┘
    ↓
CliConnector::next_event()
    ↓
Update internal state
    ↓
Return to user (stdout feedback)
```

### 2. Disconnection Flow

```
User Command
    ↓
cli::vpn::off()
    ↓
CliConnector::disconnect()
    ↓
child.kill() [SIGTERM]
    ↓
tokio::time::timeout(5s, child.wait())
    ↓
    ├─ Ok(exit_status) → Graceful disconnect
    │                     ↓
    │               ConnectionEvent::Disconnected
    │
    └─ Err(timeout) → Force kill
                      ↓
                force_kill() [SIGKILL]
                      ↓
                ConnectionEvent::Disconnected
```

### 3. Error Handling Flow

```
OpenConnect stderr output
    ↓
monitor_stderr() reads line
    ↓
OutputParser::parse_error(line)
    ↓
    ├─ Known pattern → ConnectionEvent::Error { kind, raw_output }
    │                    ↓
    │              Update state to Failed
    │                    ↓
    │              User sees error + raw output
    │
    └─ Unknown pattern → ConnectionEvent::UnknownOutput
                           ↓
                     Log at debug level (FR-015)
                           ↓
                     User sees "Unparsed error: <line>" (FR-004)
```

## Type Definitions Reference

### Supporting Types

```rust
/// VPN configuration from TOML
#[derive(Debug, Clone)]
pub struct VpnConfig {
    pub server: String,
    pub protocol: String,  // "f5"
    pub username: String,
}

/// Credentials from keyring
#[derive(Debug)]
pub struct Credentials {
    pub pin: SecureString,
    pub otp: Option<String>,  // Generated TOTP token
}

/// Secure string wrapper (zeroized on drop)
pub struct SecureString {
    inner: String,
}

/// VPN-specific errors
#[derive(Debug, thiserror::Error)]
pub enum VpnError {
    #[error("Process spawn failed: {0}")]
    ProcessSpawnError(String),

    #[error("Authentication failed: {0}")]
    AuthenticationError(String),

    #[error("Connection timeout after {0}s")]
    ConnectionTimeout(u64),

    #[error("Process termination failed: {0}")]
    TerminationError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("OpenConnect error: {0}")]
    OpenConnectError(String),
}
```

## State Machine

### CliConnector Internal State

```
[Idle] ──connect()──→ [Connecting] ──ProcessStarted──→ [Authenticating]
                                                              │
                                                              │ F5SessionEstablished
                                                              ↓
[Failed]←──Error──[Established]──disconnect()──→ [Disconnecting]
   │                    │                              │
   │                    │ TunConfigured                │ SIGTERM/SIGKILL
   │                    ↓                              ↓
   │              (no state change)              [Idle]
   │
   └──→ (terminal state, requires new CliConnector instance)
```

**State Invariants**:
- `Idle`: No child process, no events
- `Connecting`: Process spawned, stdin written
- `Authenticating`: Waiting for F5SessionEstablished event
- `Established`: TUN device up, IP assigned (connection active)
- `Disconnecting`: SIGTERM sent, waiting for graceful exit
- `Failed`: Error occurred, requires manual cleanup

## Testing Considerations

### Test Data

**Mock OpenConnect Output** (for OutputParser tests):
```text
# Success case
POST https://vpn.example.com/
Got CONNECT response: HTTP/1.1 200 OK
Connected to F5 Session Manager
Session token: 1234567890abcdef
Connected tun0 as 10.0.1.100
Configured as 10.0.1.100
Established connection

# Error case
POST https://vpn.example.com/
Failed to authenticate
fgets (stdin): Resource temporarily unavailable
```

**Mock Credentials**:
```rust
Credentials {
    pin: SecureString::from("test-pin-12345"),
    otp: Some("123456".to_string()),
}
```

### Test Doubles

```rust
// Mock OutputParser for testing CliConnector
struct MockParser {
    responses: Vec<ConnectionEvent>,
}

impl MockParser {
    fn parse_line(&self, _line: &str) -> ConnectionEvent {
        self.responses.pop().unwrap()
    }
}

// Mock process for testing without real OpenConnect
struct MockChild {
    stdout: MockStdout,
    stderr: MockStderr,
}
```

## Design Decisions

1. **Event-driven architecture**: Allows asynchronous feedback to CLI without blocking
2. **Immutable ConnectionEvent**: Simplifies concurrent event handling across tasks
3. **Arc<Mutex<>> for shared state**: Safe multi-threaded access to process/state
4. **Enum-based state machine**: Type-safe transitions, impossible states ruled out
5. **Pattern-based parsing**: Flexible for OpenConnect format variations
6. **Fallback to UnknownOutput**: Graceful degradation per FR-004
7. **SecureString wrapper**: Memory safety for credentials (future: zeroization)
8. **Separate stdout/stderr monitors**: Concurrent handling of info vs errors

## Future Extensions

1. **Multi-protocol support**: Add `OutputParser::with_protocol(Protocol)` factory
2. **Version detection**: Parse `openconnect --version` output (deferred per clarification)
3. **Connection metrics**: Add `ConnectionEvent::MetricsSnapshot { bytes_sent, bytes_recv }`
4. **Reconnection logic**: Add `CliConnector::reconnect()` method (out of scope)
5. **Structured logging**: Embed `tracing::Span` in ConnectionEvent for distributed tracing
