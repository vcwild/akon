# Research: Network Interruption Detection and Automatic Reconnection

**Date**: 2025-11-04
**Phase**: 0 (Research & Technology Selection)

## Overview

This document resolves technical unknowns identified in the Technical Context section of the implementation plan. Each decision is documented with rationale and alternatives considered.

## Research Tasks

### 1. NetworkManager D-Bus Integration Library

**Decision**: Use `zbus` (pure Rust async D-Bus library)

**Rationale**:

- **Modern async support**: Integrates naturally with tokio runtime already in use
- **Type-safe**: Generates Rust types from D-Bus introspection
- **Maintained**: Active development, latest stable version 4.x
- **Pure Rust**: No C dependencies beyond system D-Bus libraries
- **NetworkManager support**: Well-documented NetworkManager integration examples

**Alternatives Considered**:

- **dbus-rs**: Older, synchronous API would require separate thread or blocking operations
- **libdbus-sys**: Low-level FFI bindings, too verbose for this use case
- **Manual D-Bus protocol**: Overly complex, reinventing the wheel

**Implementation Notes**:

- Use `zbus::Connection::system()` to connect to system bus
- Monitor `org.freedesktop.NetworkManager` signals:
  - `StateChanged` for network up/down events
  - `PropertiesChanged` on active connection for interface changes
- Use `zbus::proxy` macro to generate typed proxy for NetworkManager

**Dependencies to Add**:

```toml
[dependencies]
zbus = "4.0"
```

---

### 2. HTTP/HTTPS Health Check Client

**Decision**: Use `reqwest` with tokio runtime

**Rationale**:

- **Async native**: Built on tokio/hyper, matches existing runtime
- **High-level API**: Simple GET request with timeout is ~5 lines of code
- **HTTPS support**: Handles TLS certificates out of the box
- **Configurable**: Easy to set timeouts, user-agents, follow redirects
- **Widely used**: Battle-tested in production Rust applications
- **Lightweight**: Can be configured with minimal features

**Alternatives Considered**:

- **hyper directly**: Lower-level, more verbose for simple GET requests
- **ureq**: Blocking API would require separate thread
- **curl-rust**: C dependency, more overhead than needed

**Implementation Notes**:

- Configure client with:
  - 5-second timeout (connection + response)
  - No redirect following (endpoint should respond directly)
  - Minimal feature set (no cookies, no compression needed)
- Accept any 2xx/3xx status code as "healthy"
- Log non-2xx responses but don't immediately fail (could be auth redirect)
- Verify through VPN tunnel by ensuring health check happens after VPN is up

**Dependencies to Add**:

```toml
[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

Note: Using `rustls-tls` instead of `native-tls` to avoid OpenSSL dependency

---

### 3. State Persistence Strategy

**Decision**: TOML file in `~/.config/akon/reconnection_state.toml`

**Rationale**:

- **Consistency**: Matches existing config file format
- **Human-readable**: Easy to inspect/debug state manually
- **Atomic writes**: Use `fs::write` with temp file + rename for crash safety
- **Simple schema**: Only need to store:
  - Current state enum variant
  - Attempt count
  - Next retry timestamp
  - Last known network state

**Alternatives Considered**:

- **JSON**: More verbose than TOML for simple key-value data
- **Binary (bincode)**: Not human-readable, harder to debug
- **SQLite**: Overkill for single-user, single-row state
- **In-memory only**: Would lose reconnection state across process restarts

**Implementation Notes**:

- Use `serde` to serialize/deserialize ConnectionState
- Load state on daemon start, save on every state transition
- Handle missing/corrupted file gracefully (treat as Disconnected)
- Include version field for future schema migrations

**Schema Example**:

```toml
version = "1.0"
state = "Reconnecting"
attempt = 3
next_retry_at = 1699104000
max_attempts = 5
```

---

### 4. Exponential Backoff Implementation

**Decision**: Custom implementation using tokio::time::sleep

**Rationale**:

- **Simple algorithm**: `interval = base * multiplier^(attempt - 1)` capped at max
- **No external dependency needed**: ~20 lines of code
- **Deterministic**: Easy to test with fixed time points
- **Integrated with tokio**: Natural async/await with sleep

**Algorithm**:

```rust
fn calculate_next_retry(attempt: u32, config: &RetryPolicy) -> Duration {
    let base_secs = config.base_interval_secs; // Default: 5
    let multiplier = config.backoff_multiplier; // Default: 2
    let max_secs = config.max_interval_secs;    // Default: 60

    let interval_secs = base_secs * multiplier.pow(attempt - 1);
    let capped_secs = interval_secs.min(max_secs);

    Duration::from_secs(capped_secs as u64)
}
```

**Test Cases**:

- Attempt 1: 5s * 2^0 = 5s
- Attempt 2: 5s * 2^1 = 10s
- Attempt 3: 5s * 2^2 = 20s
- Attempt 4: 5s * 2^3 = 40s
- Attempt 5: 5s * 2^4 = 80s → capped at 60s

**Alternatives Considered**:

- **tokio-retry crate**: Adds dependency for simple algorithm we can implement
- **Jitter**: Not needed for single-user client (no thundering herd)
- **Fibonacci backoff**: Grows slower than exponential, doesn't meet requirements

---

### 5. Background Task Architecture

**Decision**: Spawn dedicated tokio task for reconnection monitor

**Rationale**:

- **Non-blocking**: CLI commands remain responsive during reconnection
- **Async coordination**: Can await network events, health checks, timers simultaneously
- **Lifecycle management**: Task can be cancelled when user manually disconnects
- **State isolation**: Monitor task owns reconnection state, CLI queries via channel

**Architecture**:

```rust
tokio::spawn(async move {
    let mut network_events = NetworkMonitor::new().await?;
    let health_checker = HealthChecker::new(config.health_check_endpoint);
    let mut reconnection_state = ReconnectionState::load_or_default();

    loop {
        tokio::select! {
            event = network_events.next() => {
                // Handle network state change
            }
            _ = health_check_timer.tick() => {
                // Perform periodic health check
            }
            _ = reconnection_state.next_retry() => {
                // Attempt reconnection
            }
            cmd = command_rx.recv() => {
                // Handle CLI commands (status query, manual disconnect)
            }
        }
    }
});
```

**Communication**:

- Use `tokio::sync::mpsc` channel for CLI → monitor commands
- Use `tokio::sync::watch` channel for monitor → CLI state updates
- CLI reads current state from watch channel (always latest value)

**Alternatives Considered**:

- **Blocking thread**: Would need manual synchronization, less idiomatic Rust
- **Signals only**: Can't coordinate multiple async events cleanly
- **Single-threaded event loop**: CLI would block during reconnection

---

## Summary of Dependencies

New dependencies to add to `akon-core/Cargo.toml`:

```toml
[dependencies]
zbus = "4.0"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
# Existing dependencies already sufficient for:
# - tokio (timers, channels, tasks)
# - serde/toml (state persistence)
# - tracing (logging)
# - nix (signal handling)
```

Total new dependencies: **2** (zbus, reqwest)

---

## Open Questions

None - all NEEDS CLARIFICATION items from Technical Context have been resolved.

---

## Next Steps

Proceed to Phase 1: Design & Contracts

- Define data model for ConnectionState, ReconnectionPolicy, NetworkEvent
- Design contracts for NetworkMonitor, HealthChecker, ReconnectionManager
- Create quickstart.md with development setup instructions
