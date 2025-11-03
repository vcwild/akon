# Research: OpenConnect CLI Delegation Refactor

**Feature**: 002-refactor-openconnect-to
**Date**: 2025-11-03
**Purpose**: Resolve technical unknowns and establish implementation patterns for CLI-based OpenConnect integration

## Research Areas

### 1. Rust Process Spawning & Management

**Decision**: Use `tokio::process::Command` with async I/O

**Rationale**:
- Tokio provides non-blocking process APIs perfect for monitoring stdout/stderr
- Async model allows concurrent handling of process output and user interaction
- Better suited for long-running processes like VPN connections (10-30 seconds)
- Integrates with existing async patterns if present

**Alternatives Considered**:
- `std::process::Command`: Blocking I/O makes concurrent output monitoring difficult
- `async-process`: Less ecosystem support than Tokio, additional dependency
- `nix` crate for raw POSIX APIs: Overcomplicated for process spawning needs

**Implementation Pattern**:
```rust
use tokio::process::{Command, ChildStdout, ChildStderr};
use tokio::io::{AsyncBufReadExt, BufReader};

let mut child = Command::new("openconnect")
    .args(&["--protocol=f5", "--passwd-on-stdin", server_url])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

// Monitor stdout/stderr in separate tasks
let stdout = child.stdout.take().unwrap();
let stderr = child.stderr.take().unwrap();
```

### 2. OpenConnect 9.x Output Format Analysis

**Decision**: Target OpenConnect 9.12 (latest stable as of Nov 2025) with F5 protocol

**Rationale**:
- OpenConnect 9.x is widely available in Ubuntu 22.04+, Debian 12+
- F5 protocol output format is relatively stable
- Version 9.12 includes improved error reporting

**Key Output Patterns to Parse** (from manual testing):
```
# Authentication phase
"POST https://vpn.example.com/..."
"Got CONNECT response: HTTP/1.1 200 OK"

# F5 session establishment
"Connected to F5 Session Manager"
"Session token: ..."

# TUN configuration
"Connected tun0 as 10.0.1.100"
"Configured as 10.0.1.100"

# Connection established
"Established connection"

# Errors
"Failed to authenticate"
"SSL connection failure"
"Certificate validation error"
```

**Alternatives Considered**:
- Supporting multiple versions (8.0, 8.5, 9.x): Adds complexity; clarification specifies single version first
- Using structured output format: OpenConnect doesn't provide JSON/structured output
- Version detection via `openconnect --version`: Deferred per clarification decision

### 3. Line-Buffered Output Parsing (<500ms Latency)

**Decision**: Use `BufReader::lines()` with `tokio::select!` for event-driven parsing

**Rationale**:
- Line-buffered reading naturally aligns with OpenConnect's output format
- `tokio::select!` allows handling multiple streams (stdout/stderr) concurrently
- Timeout handling built into Tokio primitives
- Meets <500ms latency requirement (line arrival triggers immediate processing)

**Implementation Pattern**:
```rust
let mut stdout_reader = BufReader::new(stdout).lines();
let mut stderr_reader = BufReader::new(stderr).lines();

loop {
    tokio::select! {
        Some(line) = stdout_reader.next_line() => {
            if let Ok(line) = line {
                let event = parser.parse_line(&line);
                event_sender.send(event).await?;
            }
        }
        Some(line) = stderr_reader.next_line() => {
            if let Ok(line) = line {
                // Error stream parsing
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(60)) => {
            // Connection timeout (FR-009)
            break;
        }
    }
}
```

**Alternatives Considered**:
- Character-by-character streaming: Overcomplicated, higher overhead
- Polling with `try_wait()`: Introduces latency, less efficient
- Channel-based buffering: Adds complexity without latency benefit

### 4. Graceful Process Termination (SIGTERM→SIGKILL)

**Decision**: Use `child.kill()` with 5-second timeout then force kill

**Rationale**:
- Tokio's `Child::kill()` sends SIGTERM on Unix
- Manual SIGKILL required if process doesn't exit
- Aligns with FR-006/FR-007 requirements

**Implementation Pattern**:
```rust
// Graceful termination
child.kill().await?;

// Wait with timeout
match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
    Ok(_) => {
        // Process exited gracefully
    }
    Err(_) => {
        // Force kill
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
            nix::sys::signal::Signal::SIGKILL
        )?;
    }
}
```

**Alternatives Considered**:
- Using `std::process`: Lack of async timeout handling
- Shell script wrapper: Adds external dependency, harder to test
- No force-kill: Risk of zombie processes (violates FR-007)

### 5. Password Transmission via Stdin

**Decision**: Write password to stdin immediately after spawn, then close

**Rationale**:
- `--passwd-on-stdin` expects password on first line
- Closing stdin after write prevents accidental leakage
- Constitution-compliant secure channel

**Implementation Pattern**:
```rust
let mut stdin = child.stdin.take().unwrap();
stdin.write_all(password.as_bytes()).await?;
stdin.write_all(b"\n").await?;
stdin.flush().await?;
drop(stdin); // Close stdin
```

**Security Notes**:
- Password not visible in process arguments (`ps aux`)
- Not stored in command history
- In-memory clearance after transmission (future enhancement)

**Alternatives Considered**:
- Environment variable: Visible to other processes, violates constitution
- Temporary file: Leaves traces on disk, security risk
- Named pipe: Overcomplicated for single password transmission

### 6. Credential Backward Compatibility

**Decision**: Preserve exact keyring key names and TOML structure

**Rationale**:
- FR-010 mandates full backward compatibility
- No migration burden on users
- Existing `auth/` module interfaces unchanged

**Verified Compatibility**:
- Keyring keys: `akon:pin`, `akon:otp_seed` (unchanged)
- TOML config fields: `vpn.server`, `vpn.username`, `vpn.protocol` (unchanged)
- OTP generation: Existing TOTP algorithm unchanged

**No Changes Required**: Auth module remains untouched by this refactor

### 7. Testing Strategy: TDD with Regression Testing

**Decision**: Three-tier test categorization per FR-024

**Test Categories**:
1. **Delete**: FFI binding tests (`test_ffi_wrapper.rs`, C callback tests)
2. **Preserve**: Functional/behavioral tests (connection flow, credential handling)
3. **Update**: Tests with FFI-specific interfaces (mock FFI → mock CLI)

**TDD Workflow** (per FR-021):
```rust
// 1. RED: Write failing test
#[tokio::test]
async fn test_parse_connected_event() {
    let parser = OutputParser::new();
    let line = "Configured as 10.0.1.100";
    let event = parser.parse_line(line);
    assert!(matches!(event, ConnectionEvent::Connected { ip: _ }));
}

// 2. GREEN: Minimal implementation
impl OutputParser {
    fn parse_line(&self, line: &str) -> ConnectionEvent {
        if line.contains("Configured as") {
            // Extract IP...
            ConnectionEvent::Connected { ip }
        }
    }
}

// 3. REFACTOR: Clean up while tests stay green
```

**Mock Strategy**:
- Unit tests: Mock `OutputParser` with pre-captured OpenConnect output
- Integration tests: Mock `Command` to simulate process behavior
- System tests: Use `test-doubles` pattern for actual OpenConnect calls

### 8. Systemd Journal Logging (FR-022)

**Decision**: Use `slog` or `tracing` with systemd backend

**Rationale**:
- Structured logging with key-value pairs
- Direct systemd journal integration
- Compatible with constitution Principle IV

**Implementation Pattern**:
```rust
use tracing::{info, error};

info!(
    connection_id = %conn_id,
    server = %server_url,
    state = "F5SessionEstablished",
    "VPN state transition"
);
```

**Alternatives Considered**:
- `log` crate: Less structured, harder to query
- Custom systemd integration: Reinventing wheel
- File logging: Doesn't meet systemd journal requirement

### 9. Error Handling: OpenConnect Error Patterns

**Decision**: Pattern matching on stderr output with fallback to raw display

**Common Error Patterns**:
```
"Failed to authenticate" → Authentication failure
"SSL connection failure" → Network/TLS issue
"Certificate validation error" → Cert problem
"Failed to open tun device" → Permissions issue
"Cannot resolve hostname" → DNS/network issue
```

**Fallback Strategy** (per FR-004):
- Unrecognized errors: Display raw output with "Unparsed error:" prefix
- Logs full stderr to debug level for troubleshooting

### 10. Performance Benchmarking

**Decision**: Use `criterion` for performance regression testing

**Metrics to Track** (per Success Criteria):
- Connection establishment time (SC-001: <30s target)
- Event parsing latency (FR-003: <500ms target)
- Build time (SC-004: >50% reduction target)

**Baseline Measurements Needed**:
- Current FFI implementation build time
- Current connection time to F5 VPN
- Current test execution time

## Dependencies Summary

**New Dependencies**:
```toml
[dependencies]
tokio = { version = "1.35", features = ["process", "io-util", "time", "macros", "rt-multi-thread"] }
anyhow = "1.0"  # May already exist
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
criterion = "0.5"
```

**Removed Dependencies**:
```toml
bindgen = "..."  # REMOVE
cc = "..."       # REMOVE
```

## Open Questions / Future Work

1. **OpenConnect version detection**: Deferred per clarification; implement if users report compatibility issues
2. **Multi-version support**: Deferred; can add version-specific parsers later
3. **Connection retry logic**: Out of scope per spec
4. **Daemon mode**: Out of scope per spec
5. **In-memory password clearance**: Security enhancement for future (not blocking)

## References

- OpenConnect Documentation: https://www.infradead.org/openconnect/manual.html
- Tokio Process Documentation: https://docs.rs/tokio/latest/tokio/process/
- F5 VPN Protocol: https://www.infradead.org/openconnect/f5.html
- Rust Async Book: https://rust-lang.github.io/async-book/
