# Network Interruption Detection - Developer Quickstart

**Feature**: Automatic VPN reconnection with exponential backoff
**Branch**: `003-network-interruption-detection`
**Spec**: [spec.md](./spec.md)
**Plan**: [plan.md](./plan.md)

## Prerequisites

### System Requirements

- **OS**: Linux with NetworkManager
- **Rust**: 1.70+ (stable channel, edition 2021)
- **Services**: systemd, D-Bus, GNOME Keyring (or compatible secret service)

### Development Tools

```bash
# Rust toolchain
rustup update stable
rustup default stable

# Development tools
sudo apt install -y \
    build-essential \
    pkg-config \
    libdbus-1-dev \
    libssl-dev

# Optional: Code quality tools
rustup component add rustfmt clippy
```

### NetworkManager Setup

```bash
# Verify NetworkManager is running
systemctl status NetworkManager

# Install D-Bus introspection tools (for debugging)
sudo apt install -y d-feet busctl
```

## Getting Started

### 1. Clone and Build

```bash
cd /home/vcwild/Projects/personal/akon
git checkout 003-network-interruption-detection

# Build entire workspace
cargo build

# Build with optimizations (for testing performance)
cargo build --release
```

### 2. Install Dependencies

All dependencies are declared in `Cargo.toml`. New dependencies for this feature:

```toml
# akon-core/Cargo.toml
[dependencies]
# Network monitoring (D-Bus integration)
zbus = "4.0"

# Health checks (HTTP client)
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }

# Async runtime (already present, verify features)
tokio = { version = "1.35", features = ["sync", "time", "macros", "process", "signal"] }

# Error handling (already present)
thiserror = "1.0"

# Logging (already present)
tracing = "0.1"

# Serialization (already present)
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"

[dev-dependencies]
# HTTP mocking for health check tests
wiremock = "0.6"
```

Install dependencies:

```bash
cargo fetch
```

### 3. Configure Editor

For VS Code:

```json
{
  "rust-analyzer.cargo.features": ["all"],
  "rust-analyzer.checkOnSave.command": "clippy",
  "editor.formatOnSave": true
}
```

For vim with rust-analyzer:

```lua
-- Add to init.lua
require'lspconfig'.rust_analyzer.setup{
  settings = {
    ["rust-analyzer"] = {
      checkOnSave = {
        command = "clippy"
      }
    }
  }
}
```

## Project Structure

New modules for this feature (to be created):

```
akon-core/src/vpn/
├── network_monitor.rs       # D-Bus NetworkManager integration
├── health_check.rs          # HTTP health check client
└── reconnection.rs          # Reconnection orchestration with backoff

akon-core/tests/
├── network_monitor_tests.rs # Unit tests for NetworkMonitor
├── health_check_tests.rs    # Unit tests for HealthChecker
└── reconnection_tests.rs    # Unit tests for ReconnectionManager

specs/003-network-interruption-detection/
├── spec.md                  # Feature specification
├── plan.md                  # Implementation plan
├── research.md              # Technology research
├── data-model.md            # Data structures and state machine
├── contracts/               # Interface contracts
│   ├── network-monitor.md
│   ├── health-checker.md
│   └── reconnection-manager.md
└── quickstart.md            # This file
```

## Development Workflow

### Run Tests

```bash
# All tests
cargo test

# Specific module tests
cargo test --lib network_monitor
cargo test --lib health_check
cargo test --lib reconnection

# Integration tests only
cargo test --test '*'

# With output
cargo test -- --nocapture

# Watch mode (requires cargo-watch)
cargo watch -x test
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint with Clippy
cargo clippy -- -D warnings

# Check without building
cargo check

# Full CI pipeline
make test  # Runs format, clippy, test, coverage
```

### Run Application

```bash
# Debug build
cargo run -- vpn on

# Check VPN status
cargo run -- vpn status

# View reconnection state
cargo run -- vpn status --verbose

# Release build (faster)
cargo run --release -- vpn on
```

## Testing Strategy

### Unit Tests

Each module has unit tests with mocks:

- **NetworkMonitor**: Mock D-Bus with test fixtures
- **HealthChecker**: Mock HTTP server with wiremock
- **ReconnectionManager**: Mock NetworkMonitor + HealthChecker

Run unit tests:

```bash
cargo test --lib
```

### Integration Tests

Test with real services:

- Real NetworkManager (requires running system)
- Real HTTPS endpoints (requires network)
- Real VPN connection (requires credentials)

Run integration tests (may require sudo):

```bash
cargo test --test '*' -- --test-threads=1
```

### Manual Testing

#### Test Network Interruption

```bash
# Terminal 1: Start VPN with verbose logging
RUST_LOG=debug cargo run -- vpn on

# Terminal 2: Monitor state changes
watch -n 1 'cargo run -- vpn status'

# Terminal 3: Simulate network interruption
sudo nmcli networking off
sleep 5
sudo nmcli networking on

# Observe reconnection attempts in Terminal 1
```

#### Test Health Check Failure

```bash
# Start VPN
cargo run -- vpn on

# Block health check endpoint (simulate VPN tunnel failure)
# Add iptables rule to drop packets to health check endpoint
sudo iptables -A OUTPUT -d <health-check-ip> -j DROP

# Wait 60 seconds (health check interval)
# Observe reconnection attempts

# Restore connectivity
sudo iptables -D OUTPUT -d <health-check-ip> -j DROP
```

#### Test Suspend/Resume

```bash
# Start VPN
cargo run -- vpn on

# Suspend system
systemctl suspend

# Wait and resume
# Observe reconnection triggered by SystemResumed event
```

## Configuration

### Development Config

Create test configuration:

```bash
mkdir -p ~/.config/akon
cat > ~/.config/akon/config.toml << 'EOF'
[vpn]
gateway = "vpn.example.com"
username = "testuser"

[reconnection]
max_attempts = 5
base_interval_secs = 5
backoff_multiplier = 2.0
max_interval_secs = 60
health_check_interval_secs = 60
health_check_timeout_secs = 5
health_check_endpoint = "https://vpn.example.com/healthz"
consecutive_failures_threshold = 3
EOF
```

### Keyring Setup

Store credentials in GNOME Keyring:

```bash
# Using secret-tool
secret-tool store --label="akon VPN Password" service akon account testuser

# Or use akon setup
cargo run -- setup \
    --gateway vpn.example.com \
    --username testuser \
    --totp-secret JBSWY3DPEHPK3PXP
```

## Debugging

### Enable Debug Logging

```bash
# All modules
RUST_LOG=debug cargo run -- vpn on

# Specific modules
RUST_LOG=akon_core::vpn::network_monitor=debug,akon_core::vpn::reconnection=debug cargo run -- vpn on

# Trace level (very verbose)
RUST_LOG=trace cargo run -- vpn on
```

### View Logs with journalctl

```bash
# Real-time logs from akon
journalctl -f -t akon

# Last 100 lines
journalctl -t akon -n 100

# Since last boot
journalctl -t akon -b
```

### D-Bus Debugging

```bash
# Monitor NetworkManager signals
dbus-monitor --system "type='signal',sender='org.freedesktop.NetworkManager'"

# Inspect NetworkManager state
busctl introspect org.freedesktop.NetworkManager /org/freedesktop/NetworkManager

# Check network connectivity state
busctl get-property org.freedesktop.NetworkManager \
    /org/freedesktop/NetworkManager \
    org.freedesktop.NetworkManager \
    State
```

### HTTP Debugging

```bash
# Test health check endpoint manually
curl -v https://vpn.example.com/healthz

# Monitor HTTP traffic (requires mitmproxy)
mitmproxy --mode transparent --host

# Check TLS certificate
openssl s_client -connect vpn.example.com:443 -showcerts
```

## Common Issues

### Issue: D-Bus Connection Failed

**Symptom**: `NetworkMonitorError: DBusConnectionFailed`

**Solution**:
```bash
# Verify D-Bus is running
systemctl status dbus

# Check user session D-Bus
echo $DBUS_SESSION_BUS_ADDRESS

# Try with system bus
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/var/run/dbus/system_bus_socket
```

### Issue: Health Check Timeout

**Symptom**: Health checks always timeout

**Solution**:
```bash
# Verify endpoint is reachable
curl -v --max-time 5 https://vpn.example.com/healthz

# Check DNS resolution
dig vpn.example.com

# Test with longer timeout in config
health_check_timeout_secs = 10
```

### Issue: Max Attempts Exceeded Quickly

**Symptom**: Reconnection gives up after first failure

**Solution**:
```bash
# Verify configuration
cargo run -- vpn status --verbose | grep max_attempts

# Increase max attempts in config
max_attempts = 10

# Check backoff calculation
# Should see: 5s → 10s → 20s → 40s → 60s
```

## Performance Benchmarking

### Measure Health Check Latency

```bash
# Run health check in loop
for i in {1..100}; do
    time cargo run --release -- vpn check
done | grep real | awk '{print $2}' | sort -n
```

### Measure Reconnection Time

```bash
# Monitor reconnection duration
RUST_LOG=info cargo run -- vpn on 2>&1 | \
    grep -E 'Reconnection (initiated|successful)' | \
    awk '{print $1}' | \
    xargs -I {} date -d {} +%s | \
    awk 'NR%2==1{start=$1} NR%2==0{print $1-start "s"}'
```

## Manual Recovery Testing (T054 - User Story 4)

### Test Cleanup Command

```bash
# 1. Check for orphaned processes
ps aux | grep openconnect

# 2. Run cleanup (terminates all openconnect processes)
cargo run -- vpn cleanup

# Expected output:
# 🧹 Cleaning up orphaned OpenConnect processes...
#   ✓ Terminated 2 process(es)
# ✓ Cleanup complete

# 3. Verify processes terminated
ps aux | grep openconnect  # Should show no results

# 4. Check state file removed
ls -la /tmp/akon_vpn_state.json  # Should not exist
```

### Test Reset Command

```bash
# 1. Simulate error state (max attempts exceeded)
# This would normally happen after multiple failed reconnection attempts

# 2. Run reset
cargo run -- vpn reset

# Expected output:
# 🔄 Resetting reconnection state...
#   ℹ This feature requires integration with the VPN daemon
#   Workaround: Disconnect and reconnect:
#     1. akon vpn off
#     2. akon vpn cleanup
#     3. akon vpn on
#   ✓ Cleared connection state
# ✓ Reset complete - ready for new connection attempt

# 3. Verify can reconnect
cargo run -- vpn on
```

### Test Status with Error State

```bash
# 1. Create error state file (for testing)
cat > /tmp/akon_vpn_state.json << 'EOF'
{
  "state": "Error",
  "error": "Connection refused after 5 attempts",
  "max_attempts": 5,
  "timestamp": "2025-11-04T10:30:00Z"
}
EOF

# 2. Check status
cargo run -- vpn status

# Expected output:
# ● Status: Error - Max reconnection attempts exceeded
#   Last error: Connection refused after 5 attempts
#   ❌ Failed after 5 reconnection attempts
#
# ⚠ Manual intervention required:
#   1. Run akon vpn cleanup to terminate orphaned processes
#   2. Run akon vpn reset to clear retry counter
#   3. Run akon vpn on to reconnect

# 3. Exit code should be 3
echo $?  # Should print: 3

# 4. Cleanup
rm /tmp/akon_vpn_state.json
```

### Complete Recovery Flow

```bash
# Simulate complete manual recovery after max attempts exceeded

# 1. Check current status (should show Error)
cargo run -- vpn status

# 2. Cleanup orphaned processes
sudo cargo run -- vpn cleanup

# 3. Reset retry counters
cargo run -- vpn reset

# 4. Attempt new connection
sudo cargo run -- vpn on

# 5. Verify connected
cargo run -- vpn status
```

## Next Steps

1. **Read contracts**: Review interface contracts in `contracts/` directory (including new cleanup/reset contracts)
2. **Read data model**: Understand state machine in `data-model.md`
3. **Implement modules**: Follow TDD approach from contracts
4. **Run tests**: Verify each module with `cargo test`
5. **Integration testing**: Test with real NetworkManager
6. **Manual QA**: Follow test scenarios above including manual recovery

## Resources

- **Rust Async Book**: https://rust-lang.github.io/async-book/
- **tokio Documentation**: https://docs.rs/tokio/latest/tokio/
- **zbus Tutorial**: https://dbus2.github.io/zbus/
- **reqwest Guide**: https://docs.rs/reqwest/latest/reqwest/
- **NetworkManager D-Bus API**: https://networkmanager.dev/docs/api/latest/spec.html
- **GNOME Keyring**: https://wiki.gnome.org/Projects/GnomeKeyring
