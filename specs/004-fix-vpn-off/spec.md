# Feature Specification: Fix VPN Off Command Cleanup

**Feature Branch**: `004-fix-vpn-off`
**Created**: 2025-11-08
**Status**: Draft
**Input**: User description: "fix vpn off command to ensure cleanup of residual openconnect connections"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Clean Disconnect (Priority: P1)

When a user disconnects from VPN using `akon vpn off`, the system ensures all OpenConnect processes are terminated and no residual connections remain, eliminating the need for a separate cleanup command.

**Why this priority**: This is the core functionality fix. Users expect a disconnect command to fully clean up all VPN processes. Leaving residual connections can cause network conflicts, security issues, and confusion about connection state.

**Independent Test**: Can be fully tested by connecting to VPN, disconnecting with `akon vpn off`, then verifying no OpenConnect processes remain running on the system. Delivers immediate value by ensuring clean disconnections.

**Acceptance Scenarios**:

1. **Given** an active VPN connection with OpenConnect running, **When** user runs `akon vpn off`, **Then** the tracked OpenConnect process is terminated and no OpenConnect processes remain running
2. **Given** an active VPN connection, **When** user runs `akon vpn off` and the process doesn't respond to graceful shutdown, **Then** the system force-kills the process and verifies no OpenConnect processes remain
3. **Given** multiple orphaned OpenConnect processes from previous sessions, **When** user runs `akon vpn off`, **Then** all OpenConnect processes (tracked and orphaned) are terminated

---

### User Story 2 - Simplified Workflow (Priority: P2)

Users no longer need to remember or use a separate `akon vpn cleanup` command. The `akon vpn off` command handles all cleanup automatically, providing a single, intuitive way to disconnect.

**Why this priority**: Improves user experience by simplifying the command set. While important for usability, the cleanup functionality itself (P1) must work first.

**Independent Test**: Can be tested by attempting to use `akon vpn cleanup` command and verifying it's been removed or redirects to `akon vpn off`. User documentation should only reference `akon vpn off`.

**Acceptance Scenarios**:

1. **Given** the system is in any state, **When** user runs `akon vpn off`, **Then** all VPN cleanup is handled without requiring additional commands
2. **Given** users who previously used `akon vpn cleanup`, **When** they run `akon vpn off`, **Then** they get the same comprehensive cleanup behavior
3. **Given** the feature is complete, **When** user checks help documentation or command list, **Then** only `akon vpn off` is documented for disconnection (not `akon vpn cleanup`)

---

### User Story 3 - Reliable State Management (Priority: P3)

After running `akon vpn off`, the system state accurately reflects that no VPN connection is active, preventing confusion when users check connection status.

**Why this priority**: Important for consistency and avoiding user confusion, but depends on P1 working correctly first.

**Independent Test**: Can be tested by disconnecting with `akon vpn off`, then running `akon vpn status` to verify it reports no active connection.

**Acceptance Scenarios**:

1. **Given** an active VPN connection, **When** user runs `akon vpn off` successfully, **Then** subsequent status checks show no active connection
2. **Given** a VPN connection that was force-killed, **When** cleanup completes, **Then** state file is removed or updated to reflect disconnected state

---

### Edge Cases

- What happens when `akon vpn off` is run with no active connection? (Should gracefully report no connection and scan for orphaned processes)
- How does the system handle insufficient permissions to kill OpenConnect processes? (Should report clear error and suggest running with appropriate permissions)
- What happens if state file exists but tracked PID is already dead? (Should clean up state file and scan for any orphaned OpenConnect processes)
- How does the system handle multiple OpenConnect processes from different sources? (Should terminate all OpenConnect processes to ensure clean state)

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST terminate all OpenConnect processes when `akon vpn off` is executed, including both tracked processes and orphaned processes
- **FR-002**: System MUST attempt graceful termination (SIGTERM) before force termination (SIGKILL) for each OpenConnect process
- **FR-003**: System MUST wait a reasonable timeout period (5 seconds) for graceful shutdown before escalating to force kill
- **FR-004**: System MUST verify all OpenConnect processes are terminated after executing disconnect command
- **FR-005**: System MUST remove or update state file to reflect disconnected state after successful cleanup
- **FR-006**: System MUST handle cases where no active connection exists by scanning for and cleaning up any orphaned OpenConnect processes
- **FR-007**: System MUST provide clear feedback to users about cleanup progress (processes found, terminated, cleanup complete)
- **FR-008**: System MUST handle permission errors gracefully and provide clear guidance when unable to terminate processes
- **FR-009**: The `akon vpn cleanup` command functionality MUST be merged into `akon vpn off` command
- **FR-010**: Users MUST only need to use `akon vpn off` to disconnect and clean up VPN connections

### Key Entities

- **OpenConnect Process**: System process running the OpenConnect VPN client; may be tracked (recorded in state file) or orphaned (from previous sessions)
- **State File**: Persisted data containing current VPN connection information including process ID; must be updated or removed on disconnect
- **Connection State**: User-facing representation of VPN status (connected/disconnected); must accurately reflect actual system state

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After executing `akon vpn off`, zero OpenConnect processes remain running on the system (verified by process table scan)
- **SC-002**: Users successfully disconnect from VPN using only `akon vpn off` command without needing additional cleanup steps
- **SC-003**: System attempts graceful shutdown for 5 seconds before force-killing unresponsive processes
- **SC-004**: Connection status commands accurately report "disconnected" state after `akon vpn off` completes
- **SC-005**: Command execution completes within 10 seconds for graceful disconnects and 15 seconds for force-kill scenarios
- **SC-006**: Users report zero residual connection issues or network conflicts after disconnection (measured through user feedback and issue reports)
