# Feature Specification: Reliable `akon vpn status` for the native backend

**Feature Branch**: `007-fix-vpn-status`
**Created**: 2026-06-21
**Status**: Draft
**Input**: User description: "the akon vpn status command is not working anymore, fix it with the native backend; the state-file/PID model may not hold up — verify and control status from the foreground native solution; a state machine can control the connection flow."

## Overview

With the native, in-process F5 backend (v2.0.0), `akon vpn status` misreports the
connection. It decides "connected vs. stale" by checking whether a **PID recorded
in a state file** is still running. That model is an artifact of the previous
(openconnect) architecture, where a separate daemon process owned the tunnel.

In the native model the akon process *is* the VPN client and the **TUN interface
is the real, observable fact** of connectivity. The status command must reflect
reality: report Connected when a tunnel actually exists, and report not-connected
(or stale) otherwise — independent of any recorded PID, which may belong to a
previous session, a crashed process, or a different binary.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Accurate status while connected (Priority: P1)

As a user with an active akon VPN connection, when I run `akon vpn status` in any
terminal, I see that the VPN is **connected**, with the assigned IP, the tunnel
device, and how long it has been up.

**Why this priority**: This is the core defect — status currently reports
"inactive (stale)" while a tunnel is genuinely up, which is misleading and erodes
trust in the tool. Fixing it restores the command's basic purpose.

**Independent Test**: With a tunnel interface present and a recorded session,
running status prints an "active/connected" result and a non-zero exit code of 0.
Reproducible without a live VPN by simulating the ground-truth signal (interface
present) and a state record.

**Acceptance Scenarios**:

1. **Given** a recorded session whose tunnel interface exists, **When** I run
   `akon vpn status`, **Then** it reports Connected (exit 0) with the IP, device,
   and uptime.
2. **Given** the connection has been up for some time, **When** I run status,
   **Then** the reported uptime reflects the actual connection start time.
3. **Given** the assigned IP changed since connect, **When** I run status, **Then**
   the IP shown reflects the **current** address on the interface.

### User Story 2 - Honest status when not connected or stale (Priority: P1)

As a user, when there is no active tunnel, `akon vpn status` tells me I am not
connected — whether there was never a session, the session ended cleanly, or a
session ended uncleanly leaving a stale record.

**Why this priority**: A status command that can lie in *either* direction is
worse than none. False "connected" is as harmful as false "stale".

**Independent Test**: With no state record → reports not connected (exit 1). With
a state record but **no** matching tunnel interface → reports stale and points the
user to recovery (exit 2). Both reproducible without a live VPN.

**Acceptance Scenarios**:

1. **Given** no recorded session, **When** I run status, **Then** it reports "not
   connected" (exit 1).
2. **Given** a recorded session whose tunnel interface no longer exists, **When**
   I run status, **Then** it reports "stale" (exit 2) and suggests `akon vpn off`.
3. **Given** a stale record, **When** I then run `akon vpn off`, **Then** the
   record is cleared and a subsequent status reports "not connected".

### User Story 3 - Status is independent of who owns the process (Priority: P2)

As a user, status reflects whether a tunnel exists regardless of which process or
binary established it, so a stale or mismatched PID never causes a false result.

**Why this priority**: The current PID-based check is the root cause of the
defect. Making the tunnel interface authoritative removes a whole class of
false-negative/false-positive failures (previous session's PID, crashed owner,
upgraded binary).

**Independent Test**: With a tunnel present but the recorded PID not running,
status still reports Connected (and may note the owner is no longer running).

**Acceptance Scenarios**:

1. **Given** a tunnel interface is present but the recorded PID is dead, **When**
   I run status, **Then** it reports Connected (not stale).
2. **Given** a recorded PID that is alive but its tunnel interface is gone,
   **When** I run status, **Then** it reports stale (not connected).

### Edge Cases

- State file missing entirely → "not connected" (exit 1), no error.
- State file present but unreadable/corrupt → a clear error, not a panic.
- State file present but missing the device field → treated as stale (no tunnel
  to verify), suggest `akon vpn off`.
- Non-Linux platform (no TUN concept) → status degrades gracefully without
  crashing.
- The exit codes used by status MUST remain stable for scripting (connected vs.
  not-connected vs. stale).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `akon vpn status` MUST determine "connected" from the **existence of
  the session's tunnel network interface** (the ground truth), not from whether a
  recorded process ID is running.
- **FR-002**: When connected, status MUST display the tunnel device, the **current**
  IPv4 address on that interface, and the connection uptime/start time.
- **FR-003**: When there is no recorded session, status MUST report "not connected".
- **FR-004**: When a session is recorded but its tunnel interface is absent, status
  MUST report "stale" and direct the user to `akon vpn off` to clean up.
- **FR-005**: Status MUST NOT report "stale/not connected" solely because the
  recorded PID is not running while the tunnel interface is present.
- **FR-006**: The connection lifecycle MUST be represented by an explicit state
  machine (e.g. Disconnected → Connecting → Connected → Disconnecting, with an
  error/reconnecting path), and the persisted session record MUST be a faithful
  snapshot of that state used by status to reconcile against ground truth.
- **FR-007**: `akon vpn off` MUST clear the session record so a later status reports
  "not connected", regardless of the prior state.
- **FR-008**: Status MUST exit with stable, documented codes: connected, not
  connected, and stale must be distinguishable by exit code for scripting.
- **FR-009**: Reading the tunnel interface state for status MUST require no extra
  privilege beyond running akon normally (no sudo prompt for `status`).
- **FR-010**: Status MUST be robust to a missing, corrupt, or partial session
  record — surfacing a clear message and never panicking.

### Key Entities

- **Connection State**: the lifecycle state of the VPN (Disconnected, Connecting,
  Connected, Disconnecting, Error/Reconnecting) plus metadata: server, tunnel
  device, assigned IP, connect time, and the supervising process id.
- **Session record**: the persisted snapshot of the Connection State that survives
  across separate `akon` invocations (so `status`/`off` run from another terminal
  can observe it). It is metadata; the tunnel interface is authoritative for
  "connected".

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With an active VPN connection, `akon vpn status` reports "connected"
  with the correct IP/device/uptime 100% of the time (no false "stale").
- **SC-002**: With no active connection, `akon vpn status` reports "not connected"
  or "stale" appropriately 100% of the time (no false "connected").
- **SC-003**: Status returns within ~1 second and never requires elevated
  privileges.
- **SC-004**: A stale session (interface gone) is always recoverable with a single
  `akon vpn off`, after which status reports "not connected".
- **SC-005**: The behavior is covered by automated tests that simulate the
  ground-truth signal (interface present/absent) without requiring a live VPN,
  and they run under the project's standard `cargo test` (CI-compatible).
