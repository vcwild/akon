# Feature Specification: Network Interruption Detection and Automatic Reconnection

**Feature Branch**: `003-network-interruption-detection`
**Created**: 2025-11-04
**Status**: Draft
**Input**: User description: "in our current vpn tool implementation we are having issues because whenever we move from one network to another one or when we set the laptop to rest mode when we get back the connection is lost, but openconnect is still connected in the background. we want to implement something that would trigger a reconnection attempt if it sees that a connection is stale. but we want to make sure we have a good balance between attempts to avoid overloading the server with requests"

## Clarifications

### Session 2025-11-04

- Q: How should the health check verify that the VPN connection is actually working? → A: HTTP/HTTPS request to known endpoint through VPN tunnel
- Q: How frequently should the health check run when VPN is connected? → A: Every 60 seconds (1 minute)
- Q: How should users be notified about reconnection status? → A: Log messages to system log/journal, plus `akon vpn status` shows reconnection state
- Q: How should the system determine the network is stable enough to attempt reconnection? → A: Wait until health check endpoint is reachable
- Q: What connection states should the system track for reconnection? → A: Add `Reconnecting { attempt, next_retry_at }` state; reuse existing states for other scenarios; track attempt against configurable max retry limit

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Network Interruption Detection and Automatic Reconnection (Priority: P1)

A user experiences a network interruption (network change, WiFi disconnect, laptop suspend/resume) while the VPN is supposedly connected. The tool detects that the connection has become stale, cleans up the orphaned OpenConnect process, and automatically attempts to reconnect with intelligent retry logic to avoid overloading the VPN server.

**Why this priority**: This is the core problem being solved—detecting stale VPN connections and automatically restoring connectivity without manual intervention. Essential for MVP as it provides seamless VPN experience across network changes.

**Independent Test**: Can be fully tested by establishing a VPN connection, simulating network interruption (disconnect WiFi, switch networks, or suspend/resume laptop), and verifying: (1) the tool detects the stale connection, (2) the old OpenConnect process is terminated, (3) reconnection is attempted automatically, (4) retry attempts follow a reasonable backoff pattern.

**Acceptance Scenarios**:

1. **Given** the VPN is connected and the user disconnects from WiFi, **When** the system detects loss of network connectivity, **Then** the tool terminates the stale OpenConnect process and waits until the health check endpoint is reachable before attempting reconnection
2. **Given** the VPN is connected and the user switches from one WiFi network to another, **When** the network interface changes, **Then** the tool detects the stale connection, cleans up the old process, and automatically reconnects once the health check endpoint is reachable on the new network
3. **Given** the VPN is connected and the user puts the laptop into suspend/sleep mode, **When** the laptop resumes from suspend, **Then** the tool detects the stale connection and automatically reconnects after confirming network availability
4. **Given** the reconnection attempt fails, **When** the system schedules the next retry, **Then** it uses exponential backoff (e.g., 5s, 10s, 20s, 40s) up to a maximum interval to avoid overwhelming the server and logs the retry schedule to system log
5. **Given** multiple reconnection attempts have failed, **When** the maximum retry count is reached, **Then** the system stops automatic reconnection, logs the failure, and requires manual intervention via `akon vpn on`
6. **Given** a reconnection is in progress, **When** the user runs `akon vpn status`, **Then** the system reports the VPN state as `Reconnecting` with details about the current attempt number (e.g., "Attempt 3 of 5") and next retry time
7. **Given** the reconnection attempt number equals the configured maximum, **When** that final attempt fails, **Then** the system transitions to `Error` state with message "Reconnection failed after N attempts" and stops automatic reconnection
8. **Given** a reconnection is successful, **When** the user checks VPN status, **Then** the system accurately reports the VPN as `Connected` with the new connection details and logs the successful reconnection

---

### User Story 2 - Periodic Connection Health Check with Reconnection (Priority: P2)

A user has a VPN connection that appears active but has lost actual network connectivity (silent failure). The tool periodically verifies that the VPN connection is truly functional and triggers reconnection if connectivity checks fail consistently.

**Why this priority**: Provides defense-in-depth beyond event-based detection. Catches cases where network events aren't properly detected by the system but connectivity is actually lost. Critical for detecting silent failures that wouldn't trigger network events.

**Independent Test**: Can be tested by establishing a VPN connection, blocking network traffic at the firewall level (simulating silent network failure), and verifying the tool detects the failed health check and triggers reconnection.

**Acceptance Scenarios**:

1. **Given** the VPN appears connected but actual connectivity is lost, **When** the periodic health check runs, **Then** the tool performs an HTTP/HTTPS request to a known endpoint through the VPN tunnel, detects the connectivity failure, and initiates reconnection sequence
2. **Given** the health check fails once, **When** subsequent checks are performed, **Then** the tool requires 2-3 consecutive failures before triggering reconnection to avoid false positives
3. **Given** the VPN connection is healthy, **When** the periodic health check runs and the HTTP/HTTPS request succeeds, **Then** no action is taken and the connection remains active
4. **Given** the health check is running, **When** it performs the HTTP/HTTPS connectivity verification, **Then** the check completes within 5 seconds (including request timeout) to avoid blocking and provide timely detection
5. **Given** the health check endpoint is unreachable but the VPN tunnel is functional, **When** the check fails, **Then** the system can distinguish between endpoint failure and VPN tunnel failure to avoid unnecessary reconnections

---

### User Story 3 - Configurable Retry Policy (Priority: P2)

A user wants to customize the automatic reconnection behavior to match their usage patterns and network environment. The tool provides configurable retry policies including maximum attempts, backoff strategy, and retry intervals.

**Why this priority**: Different users have different needs—some work on stable networks where aggressive retries are fine, others on flaky networks where conservative backoff is better. Configuration enables the tool to adapt to various scenarios.

**Independent Test**: Can be tested by modifying configuration values, triggering network interruptions, and verifying the tool respects the configured retry behavior (attempts, intervals, backoff).

**Acceptance Scenarios**:

1. **Given** a user sets maximum retry attempts to 5, **When** reconnection fails 5 times, **Then** the system stops automatic reconnection and requires manual intervention
2. **Given** a user configures exponential backoff with a 2x multiplier starting at 5 seconds, **When** reconnection fails repeatedly, **Then** retry intervals follow the pattern: 5s, 10s, 20s, 40s, capped at a maximum interval
3. **Given** a user sets a maximum retry interval of 60 seconds, **When** exponential backoff would exceed this limit, **Then** subsequent retries wait exactly 60 seconds
4. **Given** a user configures the health check interval to 30 seconds, **When** the VPN is connected, **Then** health checks run every 30 seconds instead of the default 60 seconds
5. **Given** no custom configuration is provided, **When** the system performs reconnection and health checks, **Then** it uses reasonable defaults (5 max attempts, exponential backoff starting at 5s, max interval 60s, health check every 60s)

---

### User Story 4 - Manual Process Cleanup and Reset (Priority: P3)

A user wants to manually reset the VPN connection state or clean up orphaned processes when automatic recovery fails. The tool provides commands to force cleanup and reset reconnection attempt counters.

**Why this priority**: Provides a safety net and debugging capability but isn't essential for the automatic detection mechanism. Useful for edge cases and troubleshooting when automatic recovery gets stuck.

**Independent Test**: Can be tested by manually creating problematic states (orphaned processes, exceeded retry limits), then running the cleanup command and verifying it correctly resets the system to a known good state.

**Acceptance Scenarios**:

1. **Given** orphaned OpenConnect processes exist in the background, **When** the user runs the manual cleanup command, **Then** the tool identifies and terminates all OpenConnect processes and resets connection state
2. **Given** the VPN has exceeded maximum retry attempts, **When** the user runs the reset command, **Then** the retry counter is reset and automatic reconnection can resume
3. **Given** no OpenConnect processes are running, **When** the user runs the cleanup command, **Then** the tool reports that no processes were found and exits cleanly

---

### Edge Cases

- What happens when multiple OpenConnect processes are running simultaneously?
- How does the system handle race conditions between network events and reconnection attempts?
- What happens if the OpenConnect process cannot be terminated gracefully (requires SIGKILL)?
- How does the system distinguish between intentional disconnection (user-initiated) and network interruption (should reconnect)?
- What happens if network detection mechanisms are unavailable or unreliable on the system?
- How does the tool behave when running with insufficient permissions to terminate processes?
- What happens if credentials (OTP) expire during reconnection attempts?
- How does the system handle the VPN server rejecting reconnection attempts (rate limiting)?
- What happens if the user manually runs `akon vpn on` while automatic reconnection is in progress?
- How does the tool behave during rapid network changes (e.g., switching between multiple WiFi networks quickly)?
- What happens if the system is suspended during a reconnection attempt?
- What happens if the health check endpoint is temporarily unreachable but network is actually stable?
- How long should the system wait for the health check endpoint to become reachable before giving up on a reconnection cycle?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST detect network interruptions including WiFi disconnection, network interface changes, and system suspend/resume events
- **FR-002**: System MUST terminate stale OpenConnect background processes upon detecting network interruption
- **FR-003**: System MUST automatically attempt to reconnect after detecting and cleaning up a stale connection
- **FR-004**: System MUST wait for network availability before attempting reconnection after a network interruption by verifying the health check endpoint is reachable
- **FR-004a**: System MUST use the same HTTP/HTTPS health check mechanism to verify network stability as used for connection monitoring
- **FR-004b**: System MUST not count network stability checks against the reconnection attempt limit (these are pre-reconnection validation checks)
- **FR-005**: System MUST implement exponential backoff for reconnection attempts to avoid overwhelming the VPN server
- **FR-006**: System MUST enforce a maximum number of reconnection attempts before requiring manual intervention
- **FR-007**: System MUST verify that OpenConnect processes are fully terminated and not left in a zombie state
- **FR-008**: System MUST update internal VPN connection state throughout the reconnection lifecycle using defined states: `Disconnected`, `Connecting`, `Connected(metadata)`, `Disconnecting`, `Reconnecting { attempt, next_retry_at }`, `Error(message)`
- **FR-008a**: System MUST transition to `Reconnecting` state when automatic reconnection begins, including current attempt number (starting at 1) and next retry timestamp
- **FR-008b**: System MUST compare current attempt number against configured maximum attempts to determine if reconnection should continue or transition to `Error("Reconnection failed after N attempts")`
- **FR-008c**: System MUST use `Disconnected` state when waiting for network stability (health check endpoint reachability) before entering `Reconnecting` state
- **FR-009**: System MUST perform periodic connectivity health checks when VPN appears connected using HTTP/HTTPS requests to a known endpoint through the VPN tunnel
- **FR-009a**: System MUST run health checks every 60 seconds (1 minute) by default when VPN is connected
- **FR-009b**: System MUST allow configuration of the health check endpoint URL and interval to accommodate different network environments
- **FR-009c**: System MUST complete health check HTTP/HTTPS requests within 5 seconds including timeout handling
- **FR-010**: System MUST require multiple consecutive health check failures before triggering reconnection to avoid false positives
- **FR-010a**: System MUST distinguish between health check endpoint failure and VPN tunnel failure to prevent unnecessary reconnections when only the endpoint is unreachable
- **FR-011**: System MUST distinguish between temporary network fluctuations and sustained connectivity loss
- **FR-012**: System MUST handle cases where multiple OpenConnect processes exist
- **FR-013**: System MUST attempt graceful process termination (SIGTERM) before forceful termination (SIGKILL)
- **FR-014**: System MUST log all reconnection events to system log/journal with details (reason, attempt number, success/failure, next retry interval, timestamp)
- **FR-014a**: System MUST log health check results including successes, failures, and consecutive failure counts
- **FR-014b**: System MUST report current reconnection state via `akon vpn status` command including status (reconnecting/connected/disconnected), attempt number, and next retry time when applicable
- **FR-015**: System MUST NOT trigger reconnection when disconnection is intentional (user-initiated via `vpn off`)
- **FR-016**: System MUST distinguish between user-initiated disconnection and network-triggered disconnection
- **FR-017**: Users MUST be able to manually trigger process cleanup and retry counter reset via dedicated commands
- **FR-018**: Users MUST be able to configure reconnection behavior (max attempts, backoff multiplier, max interval, consecutive failures threshold, health check interval)
- **FR-019**: System MUST provide reasonable default values for all configurable reconnection parameters
- **FR-020**: System MUST handle scenarios where it lacks permission to terminate processes with clear error messages
- **FR-021**: System MUST regenerate fresh OTP tokens for each reconnection attempt
- **FR-022**: System MUST cancel any in-progress reconnection attempt when user manually initiates connection or disconnection

### Key Entities

- **Network Event**: Represents a change in network state (disconnection, interface change, suspend/resume) that triggers detection logic
- **Connection Monitor**: Tracks OpenConnect background processes, their associated connection state, reconnection attempt count, next retry time, and monitors for staleness
- **Connection State**: Enumeration of possible VPN states: `Disconnected`, `Connecting`, `Connected(metadata)`, `Disconnecting`, `Reconnecting { attempt, next_retry_at }`, `Error(message)`. The `Reconnecting` state includes current attempt number (1-indexed) and timestamp for next retry, which can be compared against configured maximum attempts to determine if retries are exhausted
- **Health Check**: Periodic verification that validates actual connectivity through the VPN tunnel using HTTP/HTTPS requests to a configured endpoint, with consecutive failure tracking and timeout handling
- **Reconnection Policy**: Configuration defining retry behavior (max attempts, backoff strategy, intervals, failure threshold, health check interval)
- **Reconnection Attempt**: Individual attempt to re-establish VPN connection, including attempt number, timestamp, and outcome
- **Cleanup Operation**: Action that terminates stale OpenConnect processes and resets connection state for reconnection

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: When a user disconnects WiFi or switches networks while VPN is connected, the system detects the stale connection and initiates reconnection within 10 seconds of the health check endpoint becoming reachable
- **SC-002**: When a user suspends and resumes their laptop, the VPN reconnects automatically within 15 seconds of resume (assuming network is available)
- **SC-003**: After network interruption, running `akon vpn status` accurately reflects the current state using the defined state model (`Disconnected`, `Reconnecting { attempt, next_retry_at }`, `Connected`, `Error`) with no stale "connected" states
- **SC-003b**: When in `Reconnecting` state, status displays current attempt number and maximum attempts (e.g., "Attempt 3 of 5") and next retry time
- **SC-003a**: All reconnection events (detected stale connection, cleanup, retry attempts, success, failure) are logged to system log/journal with sufficient detail for troubleshooting
- **SC-004**: Reconnection attempts follow configured exponential backoff pattern—measured retry intervals match the specified formula (e.g., 5s, 10s, 20s, 40s)
- **SC-005**: System stops automatic reconnection after reaching maximum configured attempts and requires manual intervention
- **SC-006**: When periodic health checks detect sustained connectivity failure (2-3 consecutive failures), reconnection is triggered within one check interval after the threshold is met
- **SC-007**: Manual cleanup command successfully identifies and terminates all orphaned OpenConnect processes in 100% of test cases
- **SC-008**: Users experience seamless VPN connectivity across network changes—95% of network interruptions result in successful automatic reconnection
- **SC-009**: VPN server is not overwhelmed—reconnection attempts never exceed configured rate limits (e.g., maximum 1 attempt per configured backoff interval)
- **SC-010**: Reconnection logic generates fresh OTP tokens for each attempt—no authentication failures due to token reuse

## Assumptions

- The system has access to network state change notifications through the operating system (e.g., NetworkManager D-Bus signals on Linux)
- The tool runs with sufficient permissions to terminate OpenConnect processes (typically the same user that started them)
- OpenConnect processes can be reliably identified by process name and/or PID tracking
- The tool maintains state about which OpenConnect processes it has started and their reconnection status
- Network interruption detection combined with health checks provides reliable stale connection detection
- Users prefer automatic reconnection over manual intervention after network changes
- The VPN server can handle reasonable reconnection rates defined by exponential backoff (e.g., 5s minimum interval)
- OTP tokens remain valid long enough for reconnection attempts (typically 30-second TOTP windows)
- The primary use case is Linux systems with GNOME Keyring (as established in previous specs)
- Configuration for reconnection policy is stored in the same config file as other VPN settings
- Default reconnection parameters provide a good balance for most users (5 attempts, exponential backoff starting at 5s, max 60s interval, 2-3 consecutive health check failures, 60s health check interval)
- A reliable health check endpoint is available and accessible through the VPN tunnel (e.g., internal corporate service, VPN gateway status page, or configured URL)
- The health check endpoint has sufficient uptime and reliability to distinguish VPN tunnel failures from endpoint failures
