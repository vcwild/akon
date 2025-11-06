# End-to-End Validation Plan

**Feature**: 003 - Network Interruption Detection and Automatic Reconnection
**Date**: 2025-11-04
**Updated**: 2025-11-05
**Status**: ⚠️ **BLOCKED - INTEGRATION GAP FOUND**

---

## Overview

This document provides a comprehensive end-to-end validation plan to verify Feature 003 is production-ready.

**⚠️ CRITICAL DISCOVERY (2025-11-05)**: E2E testing revealed that the ReconnectionManager is implemented but **not integrated** into the CLI. Task T025 was deferred. See `E2E-VALIDATION-RESULTS-PHASE4.md` for complete analysis.

**Current Status**: Phases 1-3 complete, Phase 4+ blocked until ReconnectionManager integration is completed.---

## Prerequisites

### 1. VPN Access
- ✅ VPN server accessible
- ✅ Valid credentials (username, TOTP secret)
- ✅ Network connectivity

### 2. System Requirements
- ✅ Linux with NetworkManager
- ✅ D-Bus service running
- ✅ GNOME Keyring (or compatible)
- ✅ OpenConnect installed

### 3. Testing Tools
```bash
# Install monitoring tools
sudo apt install -y \
    sysstat \
    nethogs \
    iptables \
    net-tools

# Verify tools
which pidstat nmcli iptables netstat
```

### 4. Test Environment
```bash
# Build release version for accurate performance
cargo build --release

# Create test config directory
mkdir -p ~/.config/akon-test

# Backup existing config if present
[ -f ~/.config/akon/config.toml ] && \
    cp ~/.config/akon/config.toml ~/.config/akon/config.toml.backup
```

---

## Validation Phases

### Phase 1: Setup and Configuration ✓

**Objective**: Verify interactive setup with reconnection configuration

#### Test 1.1: Basic Setup with Reconnection (T044)
```bash
# Run interactive setup
./target/release/akon setup

# User inputs:
# - VPN server: <your-vpn-server>
# - Username: <your-username>
# - Protocol: AnyConnect
# - Timeout: 30
# - DTLS: No
# - Lazy mode: No
# - TOTP Secret: <your-secret>
# - Configure reconnection: Yes
# - Health endpoint: https://www.google.com
# - Advanced settings: No
```

**Expected**:
- ✅ Config file created at `~/.config/akon/config.toml`
- ✅ File contains `[vpn]` section
- ✅ File contains `[reconnection]` section with defaults
- ✅ TOTP secret stored in keyring

**Verification**:
```bash
cat ~/.config/akon/config.toml
# Should show [reconnection] section with:
# max_attempts = 5
# base_interval_secs = 5
# backoff_multiplier = 2
# max_interval_secs = 60
# consecutive_failures_threshold = 3
# health_check_interval_secs = 60
# health_check_endpoint = "https://www.google.com"
```

#### Test 1.2: Advanced Reconnection Configuration (T044)
```bash
# Run setup again with advanced settings
./target/release/akon setup

# When prompted:
# - Overwrite: Yes
# - ... (same VPN settings) ...
# - Configure reconnection: Yes
# - Health endpoint: https://vpn-gateway.example.com/health
# - Advanced settings: Yes
#   - Max attempts: 10
#   - Base interval: 10
#   - Backoff multiplier: 3
#   - Max interval: 120
#   - Consecutive failures: 5
#   - Health check interval: 30
```

**Expected**:
- ✅ Config updated with custom values
- ✅ Validation passes for all inputs

**Verification**:
```bash
grep -A 10 "\[reconnection\]" ~/.config/akon/config.toml
# Should show custom values
```

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 2: Config Integration (T040)

**Objective**: Verify configuration values are loaded and applied correctly

#### Test 2.1: Load Config with Custom Values
```bash
# Verify config loads without errors
./target/release/akon vpn status

# Should load config and show disconnected state
```

**Expected**:
- ✅ Config loads successfully
- ✅ No validation errors
- ✅ Reconnection policy applied

#### Test 2.2: Verify Backoff Calculation
```bash
# Edit config to test backoff math
cat > ~/.config/akon/config.toml << EOF
[vpn]
server = "vpn.example.com"
username = "testuser"

[reconnection]
max_attempts = 5
base_interval_secs = 10
backoff_multiplier = 3
max_interval_secs = 200
consecutive_failures_threshold = 3
health_check_interval_secs = 60
health_check_endpoint = "https://www.google.com"
EOF

# This will be verified in reconnection tests
# Expected intervals: 10s, 30s, 90s, 200s (capped), 200s
```

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 3: Basic VPN Operations

**Objective**: Verify basic VPN connection functionality

#### Test 3.1: Connect to VPN
```bash
# Start VPN connection
RUST_LOG=info ./target/release/akon vpn on

# Wait for connection
sleep 10

# Verify connected
./target/release/akon vpn status
```

**Expected**:
- ✅ Connection establishes successfully
- ✅ OpenConnect process running
- ✅ Status shows "Connected"
- ✅ Connection metadata present (server, username, timestamp)

**Verification**:
```bash
# Check OpenConnect process
ps aux | grep openconnect

# Check network routes
ip route | grep tun

# Test connectivity through VPN
curl -I https://www.google.com
```

#### Test 3.2: Disconnect from VPN
```bash
# Disconnect
./target/release/akon vpn off

# Verify disconnected
./target/release/akon vpn status
```

**Expected**:
- ✅ OpenConnect process terminated
- ✅ Status shows "Disconnected"
- ✅ VPN routes removed

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 4: Network Interruption Detection (User Story 1)

**Objective**: Verify automatic reconnection after network changes

#### Test 4.1: WiFi Disconnect/Reconnect
```bash
# Terminal 1: Start VPN with logging
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee network_test.log

# Terminal 2: Monitor status
watch -n 1 './target/release/akon vpn status'

# Terminal 3: Simulate network interruption
# Record start time
date +%s.%N > /tmp/event_time.txt

# Disconnect WiFi
sudo nmcli networking off

# Wait 5 seconds
sleep 5

# Reconnect WiFi
sudo nmcli networking on

# Monitor reconnection in Terminal 1 logs
```

**Expected Behavior** (SC-001):
- ✅ Network event detected within 1 second
- ✅ VPN marked as "Disconnected"
- ✅ Health check endpoint reachability verified
- ✅ Reconnection initiated within 10 seconds of endpoint becoming reachable
- ✅ First attempt at 5s delay
- ✅ Connection re-established successfully

**Verification**:
```bash
# Check logs for timing
grep "network event" network_test.log
grep "Reconnection attempt" network_test.log
grep "Connected" network_test.log

# Verify timing meets requirement
# Event to reconnection: < 10s after network stable
```

#### Test 4.2: Network Interface Change
```bash
# If you have multiple network interfaces (WiFi + Ethernet)

# Terminal 1: VPN running
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee interface_test.log

# Terminal 2: Switch network interface
# Disconnect WiFi, connect Ethernet (or vice versa)
sudo nmcli con down <wifi-name>
sleep 2
sudo nmcli con up <ethernet-name>
```

**Expected** (SC-001, scenario 2):
- ✅ Interface change detected
- ✅ Old connection cleaned up
- ✅ Reconnection on new interface

#### Test 4.3: System Suspend/Resume
```bash
# Terminal 1: VPN running
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee suspend_test.log

# Terminal 2: Suspend system
sudo systemctl suspend

# Wait for system to suspend, then resume manually

# Check logs after resume
```

**Expected** (SC-002):
- ✅ Suspend event detected
- ✅ Reconnection triggered after resume
- ✅ Connection re-established within 15 seconds of resume

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 5: Exponential Backoff (User Story 1)

**Objective**: Verify backoff intervals match configuration

#### Test 5.1: Measure Backoff Timing
```bash
# Edit config for testable intervals
cat > ~/.config/akon/config.toml << EOF
[vpn]
server = "vpn.example.com"
username = "user"

[reconnection]
max_attempts = 5
base_interval_secs = 5
backoff_multiplier = 2
max_interval_secs = 60
consecutive_failures_threshold = 3
health_check_interval_secs = 60
health_check_endpoint = "https://www.google.com"
EOF

# Terminal 1: Start VPN
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee backoff_test.log

# Terminal 2: Block VPN to force reconnection attempts
# Block VPN server
sudo iptables -A OUTPUT -d <vpn-server-ip> -j DROP

# Wait for multiple attempts
sleep 180

# Restore connectivity
sudo iptables -D OUTPUT -d <vpn-server-ip> -j DROP
```

**Expected** (SC-004):
- ✅ Attempt 1 after ~5 seconds
- ✅ Attempt 2 after ~10 seconds (5 × 2^1)
- ✅ Attempt 3 after ~20 seconds (5 × 2^2)
- ✅ Attempt 4 after ~40 seconds (5 × 2^3)
- ✅ Attempt 5 after ~60 seconds (capped at max_interval)

**Verification**:
```bash
# Extract timestamps from logs
grep "Reconnection attempt" backoff_test.log | \
    awk '{print $1}' > attempt_times.txt

# Calculate intervals
awk 'NR>1{print ($1 - prev)}; {prev=$1}' attempt_times.txt

# Verify intervals within ±500ms tolerance
```

#### Test 5.2: Max Attempts Enforcement
```bash
# Continue from Test 5.1 with VPN still blocked

# Wait for max attempts to be exhausted
sleep 180

# Check final state
./target/release/akon vpn status
```

**Expected** (SC-005):
- ✅ Exactly 5 reconnection attempts made
- ✅ Status transitions to "Error" state
- ✅ Message: "Max reconnection attempts (5) exceeded"
- ✅ No further automatic attempts

**Verification**:
```bash
# Count attempts in log
grep -c "Reconnection attempt" backoff_test.log
# Should be exactly 5

# Verify error state
./target/release/akon vpn status | grep "Error"
```

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 6: Health Check Detection (User Story 2)

**Objective**: Verify periodic health checks trigger reconnection

#### Test 6.1: Health Check Success
```bash
# Start VPN with short check interval
cat > ~/.config/akon/config.toml << EOF
[vpn]
server = "vpn.example.com"
username = "user"

[reconnection]
health_check_interval_secs = 10
health_check_endpoint = "https://www.google.com"
consecutive_failures_threshold = 3
EOF

# Start VPN
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee health_test.log

# Wait 60 seconds for multiple checks
sleep 60

# Verify checks running
grep "Health check" health_test.log
```

**Expected** (SC-006, scenario 3):
- ✅ Health checks run every 10 seconds
- ✅ All checks succeed (status 200)
- ✅ No reconnection triggered
- ✅ Connection remains stable

#### Test 6.2: Health Check Consecutive Failures
```bash
# VPN still running from Test 6.1

# Terminal 2: Block health endpoint
sudo iptables -A OUTPUT -d 142.250.80.46 -p tcp --dport 443 -j DROP

# Wait for 3 consecutive failures (3 × 10s = 30s)
sleep 35

# Check logs
grep "Health check failed" health_test.log
grep "consecutive failures" health_test.log
```

**Expected** (SC-006):
- ✅ Health check fails 3 times consecutively
- ✅ Reconnection triggered after threshold met
- ✅ Reconnection within one check interval (10s) after 3rd failure

**Verification**:
```bash
# Count consecutive failures before reconnection
grep "Health check failed" health_test.log | head -3

# Verify reconnection triggered
grep "triggering reconnection" health_test.log
```

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 7: Manual Recovery (User Story 4)

**Objective**: Verify cleanup and reset commands

#### Test 7.1: Cleanup Orphaned Processes
```bash
# Simulate orphaned process
# Kill akon but leave openconnect running
./target/release/akon vpn on
sleep 5
killall akon  # Don't use vpn off

# Verify openconnect still running
ps aux | grep openconnect

# Run cleanup
sudo ./target/release/akon vpn cleanup

# Verify process terminated
ps aux | grep openconnect
# Should show no results
```

**Expected** (SC-007):
- ✅ Cleanup identifies orphaned openconnect processes
- ✅ Sends SIGTERM first
- ✅ Waits 5 seconds
- ✅ Sends SIGKILL if still alive
- ✅ Reports count of terminated processes
- ✅ Resets connection state

#### Test 7.2: Reset Retry Counter
```bash
# Force error state (from Phase 5 max attempts test)
./target/release/akon vpn status
# Should show Error state

# Reset
./target/release/akon vpn reset

# Verify state cleared
./target/release/akon vpn status
# Should show Disconnected or ready to connect
```

**Expected**:
- ✅ Retry counter reset to 0
- ✅ Error state cleared
- ✅ Can attempt new connection

#### Test 7.3: Status Error Detection
```bash
# Verify status command detects error state
# (should already be in error state from previous tests)

./target/release/akon vpn status
```

**Expected**:
- ✅ Status shows "Error" state
- ✅ Displays error message
- ✅ Suggests manual intervention
- ✅ Provides cleanup/reset commands
- ✅ Exit code is 3

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

### Phase 8: Performance Testing (T060)

**Objective**: Verify performance characteristics

#### Test 8.1: CPU Overhead
```bash
# Start VPN
./target/release/akon vpn on

# Monitor CPU for 10 minutes
pidstat -p $(pgrep -x akon) 1 600 > cpu_usage.log &

# Let run for 10 minutes
sleep 600

# Analyze results
awk '{sum+=$8} END {print "Average CPU:", sum/NR "%"}' cpu_usage.log
```

**Expected**:
- ✅ Average CPU < 0.1% when idle
- ✅ Spikes only during health checks (every 60s)

#### Test 8.2: Event Detection Latency
```bash
# Start VPN with precise logging
RUST_LOG=debug ./target/release/akon vpn on 2>&1 | tee latency_test.log &

# Trigger event with timestamp
EVENT_TIME=$(date +%s.%N)
sudo nmcli networking off
sleep 2
sudo nmcli networking on

# Wait for detection
sleep 5

# Calculate latency
DETECT_TIME=$(grep "network event detected" latency_test.log | head -1 | awk '{print $1}')
echo "Latency: $(echo "$DETECT_TIME - $EVENT_TIME" | bc)s"
```

**Expected**:
- ✅ Detection latency < 1 second
- ✅ Typically < 100ms (D-Bus is fast)

#### Test 8.3: Memory Usage
```bash
# Start VPN
./target/release/akon vpn on

# Get PID
PID=$(pgrep -x akon)

# Sample memory every 10 seconds for 1 hour
for i in {1..360}; do
    ps -o rss= -p $PID | tee -a memory_usage.log
    sleep 10
done

# Find peak
sort -n memory_usage.log | tail -1
```

**Expected**:
- ✅ Peak RSS < 5MB (5120 KB)
- ✅ Stable over time (no leaks)

#### Test 8.4: Timer Accuracy
```bash
# Already measured in Phase 5 Test 5.1
# Verify from backoff_test.log

# Calculate drift for each interval
grep "Reconnection attempt" backoff_test.log | \
    awk '{print $1}' | \
    awk 'NR>1{print ($1 - prev) - expected}; {prev=$1; expected=5*2^(NR-1)}'

# Each line shows drift from expected interval
```

**Expected**:
- ✅ Drift < 500ms per interval
- ✅ Typically < 100ms (tokio is precise)

**Status**: ⬜ Not Started | 🔄 In Progress | ✅ Passed | ❌ Failed

---

## Acceptance Criteria Verification

### User Story 1: Network Interruption (8 scenarios)

- [ ] SC 1.1: WiFi disconnect → reconnect within 10s
- [ ] SC 1.2: Network switch → cleanup + reconnect
- [ ] SC 1.3: Suspend/resume → reconnect within 15s
- [ ] SC 1.4: Backoff intervals: 5s, 10s, 20s, 40s
- [ ] SC 1.5: Max attempts → stop + Error state
- [ ] SC 1.6: Status shows "Reconnecting" with attempt X/Y
- [ ] SC 1.7: Max attempts exceeded → Error state
- [ ] SC 1.8: Successful reconnection → Status "Connected"

### User Story 2: Health Checks (5 scenarios)

- [ ] SC 2.1: Health check detects silent failure
- [ ] SC 2.2: 2-3 consecutive failures before reconnect
- [ ] SC 2.3: Success → no action, stay connected
- [ ] SC 2.4: Check completes within 5 seconds
- [ ] SC 2.5: Distinguish endpoint vs VPN failure

### User Story 3: Configuration (5 scenarios)

- [ ] SC 3.1: Max attempts config respected
- [ ] SC 3.2: Backoff multiplier config applied
- [ ] SC 3.3: Max interval cap enforced
- [ ] SC 3.4: Health check interval respected
- [ ] SC 3.5: Defaults used when config missing

### User Story 4: Manual Recovery (3 scenarios)

- [ ] SC 4.1: Cleanup terminates all openconnect processes
- [ ] SC 4.2: Reset clears retry counter
- [ ] SC 4.3: Cleanup with no processes exits cleanly

---

## Test Execution Log

| Phase | Test | Started | Completed | Status | Notes |
|-------|------|---------|-----------|--------|-------|
| 1 | Setup Basic | | | ⬜ | |
| 1 | Setup Advanced | | | ⬜ | |
| 2 | Config Load | | | ⬜ | |
| 2 | Backoff Calc | | | ⬜ | |
| 3 | VPN Connect | | | ⬜ | |
| 3 | VPN Disconnect | | | ⬜ | |
| 4 | WiFi Change | | | ⬜ | |
| 4 | Interface Change | | | ⬜ | |
| 4 | Suspend/Resume | | | ⬜ | |
| 5 | Backoff Timing | | | ⬜ | |
| 5 | Max Attempts | | | ⬜ | |
| 6 | Health Success | | | ⬜ | |
| 6 | Health Failures | | | ⬜ | |
| 7 | Cleanup | | | ⬜ | |
| 7 | Reset | | | ⬜ | |
| 7 | Status Error | | | ⬜ | |
| 8 | CPU Overhead | | | ⬜ | |
| 8 | Latency | | | ⬜ | |
| 8 | Memory | | | ⬜ | |
| 8 | Timer Accuracy | | | ⬜ | |

---

## Issues and Resolutions

| Issue ID | Description | Impact | Resolution | Status |
|----------|-------------|--------|------------|--------|
| | | | | |

---

## Sign-Off

**Validation completed by**: _____________
**Date**: _____________
**Result**: ⬜ Pass | ⬜ Fail
**Notes**:

---

## Next Steps After Validation

1. [ ] Document all test results
2. [ ] File issues for any failures
3. [ ] Update PHASE-7-COMPLETION-REPORT.md
4. [ ] Create production deployment plan
5. [ ] Merge feature branch to main
6. [ ] Tag release version
7. [ ] Deploy to production

---

**Ready to start**: Execute tests in order, document results in execution log above.
