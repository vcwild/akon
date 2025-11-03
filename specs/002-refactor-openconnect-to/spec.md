# Feature Specification: OpenConnect CLI Delegation Refactor

**Feature Branch**: `002-refactor-openconnect-to`
**Created**: 2025-10-15
**Status**: Draft
**Input**: User description: "Refactor OpenConnect to CLI delegation - Replace FFI bindings with direct CLI process management for better control and maintainability"

## Clarifications

### Session 2025-11-03
- Q: How should the migration handle the FFI removal given requirement to remove FFI entirely and allow breaking changes? → A: Immediate removal - Delete all FFI code, dependencies, and C wrappers in one commit (no backward compatibility, breaking change accepted)
- Q: How should existing tests be handled with immediate FFI removal and breaking changes? → A: Sophisticated approach - Only delete tests directly testing FFI bindings/internals; preserve and maintain functional/behavioral tests (regression testing). If test interface changes due to CLI implementation, update test to verify same behavior with new interface.
- Q: When should security audit occur given immediate FFI removal eliminates "before FFI removal" phase? → A: Remove FR-023 security audit requirement entirely
- Q: How should credential management be handled with complete rewrite and breaking changes? → A: Full compatibility - Keep existing keyring keys, TOML config format unchanged (credentials remain accessible without re-setup)
- Q: How should OpenConnect version compatibility be handled for CLI implementation? → A: Single version first - Target one version (latest stable), add version detection later if needed

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Basic VPN Connection (Priority: P1)

As a user, I want to connect to my VPN server using the `akon vpn on` command, and the system should automatically handle the OpenConnect CLI process, showing me clear progress updates until the connection is established.

**Why this priority**: Core functionality - without this, the tool cannot establish VPN connections at all. This is the minimum viable product.

**Independent Test**: Can be fully tested by running `akon vpn on` with valid credentials and verifying that connection completes successfully with "Connected" status message and assigned IP address displayed.

**Acceptance Scenarios**:

1. **Given** user has valid credentials stored in keyring and OpenConnect CLI is installed, **When** user runs `akon vpn on`, **Then** system starts OpenConnect process, displays authentication progress, and shows "Connected" with assigned IP address
2. **Given** user enters incorrect credentials, **When** system attempts connection, **Then** authentication fails with clear error message and process terminates gracefully
3. **Given** OpenConnect CLI is not installed on system, **When** user runs `akon vpn on`, **Then** system displays helpful error message indicating OpenConnect CLI is required with installation instructions

---

### User Story 2 - Connection State Tracking (Priority: P1)

As a user, I want to see real-time progress updates during VPN connection establishment so I understand what's happening and can identify issues quickly.

**Why this priority**: Critical for user experience and debugging - users need feedback during connection process which can take 10-30 seconds. Without this, users don't know if the system is working or hung.

**Independent Test**: Can be tested by monitoring terminal output during `akon vpn on` execution and verifying that each connection phase (authenticating, establishing F5 VPN session, configuring TUN, connected) is displayed with appropriate messages.

**Acceptance Scenarios**:

1. **Given** VPN connection is in progress, **When** OpenConnect outputs "Authenticating...", **Then** system displays "Authenticating with server..." to user
2. **Given** F5 VPN session is established, **When** OpenConnect outputs session confirmation, **Then** system displays "✓ Secure connection established"
3. **Given** IP address is assigned, **When** OpenConnect outputs "Configured as X.X.X.X", **Then** system displays "✓ VPN connected - IP: X.X.X.X"
4. **Given** connection fails during any phase, **When** OpenConnect outputs error message, **Then** system displays user-friendly error explanation and exits with non-zero status

---

### User Story 3 - Connection Completion Detection (Priority: P1)

As a user, I want the system to detect when my VPN connection is fully established so the CLI returns control to me promptly without manual intervention.

**Why this priority**: Core functionality - users need automated detection of connection success to know when they can proceed with their work. Manual detection would require users to interpret OpenConnect output directly, defeating the tool's purpose.

**Independent Test**: Can be tested by measuring time between connection request and CLI prompt return, verifying it occurs within 3 seconds of actual connection establishment (not 30+ seconds later).

**Acceptance Scenarios**:

1. **Given** OpenConnect outputs "Connected" or "Configured as", **When** system detects this output, **Then** connection is marked as established and success message is displayed within 2 seconds
2. **Given** connection times out after 60 seconds, **When** no success indicators are detected, **Then** system terminates OpenConnect process and displays timeout error
3. **Given** connection is interrupted during establishment, **When** OpenConnect exits unexpectedly, **Then** system detects process termination and displays disconnection message

---

### User Story 4 - Graceful Disconnection (Priority: P2)

As a user, I want to disconnect from VPN cleanly using `akon vpn off` or Ctrl+C so system resources are freed properly and no zombie processes remain.

**Why this priority**: Important for usability and system hygiene, but not blocking MVP since users can manually kill processes if needed.

**Independent Test**: Can be tested by establishing connection, running disconnect command, and verifying OpenConnect process is terminated and no orphaned processes remain.

**Acceptance Scenarios**:

1. **Given** VPN connection is active, **When** user runs `akon vpn off`, **Then** OpenConnect process receives termination signal and exits within 5 seconds, displaying "Disconnected" message
2. **Given** VPN connection is active, **When** user presses Ctrl+C, **Then** system catches interrupt signal, terminates OpenConnect gracefully, and displays "Disconnected by user"
3. **Given** OpenConnect process is unresponsive, **When** graceful termination times out after 5 seconds, **Then** system force-kills process and warns user about ungraceful shutdown

---

### User Story 5 - Connection Status Query (Priority: P3)

As a user, I want to check if my VPN is currently connected using `akon vpn status` so I can verify connection state without disrupting active connections.

**Why this priority**: Nice-to-have feature that improves user experience but is not essential - users can check connection state through OS tools.

**Independent Test**: Can be tested by running `akon vpn status` in various states (connected, disconnected, connecting) and verifying correct status is displayed.

**Acceptance Scenarios**:

1. **Given** VPN is connected, **When** user runs `akon vpn status`, **Then** system displays "Connected - IP: X.X.X.X, Duration: Xm Ys"
2. **Given** VPN is not connected, **When** user runs `akon vpn status`, **Then** system displays "Not connected"
3. **Given** VPN is connecting, **When** user runs `akon vpn status`, **Then** system displays "Connecting... (Xms elapsed)"

---

### User Story 6 - Error Recovery and Diagnostics (Priority: P3)

As a user, when connection fails, I want clear diagnostic information explaining what went wrong and suggesting fixes so I can resolve issues independently.

**Why this priority**: Enhances user experience but not blocking - basic error messages from P1 stories are sufficient for MVP.

**Independent Test**: Can be tested by triggering various failure scenarios (wrong credentials, network issues, missing dependencies) and verifying helpful error messages with actionable suggestions.

**Acceptance Scenarios**:

1. **Given** OpenConnect is not installed, **When** connection attempt fails, **Then** error message includes installation instructions for user's OS
2. **Given** network is unreachable, **When** connection fails, **Then** error explains network connectivity issue and suggests checking firewall/proxy settings
3. **Given** OpenConnect outputs unfamiliar error, **When** system cannot parse error, **Then** raw OpenConnect output is displayed with prefix indicating unparsed error

---

### Edge Cases

- What happens when OpenConnect CLI produces unexpected output format (different version)?
  - System should fall back to displaying raw output when parsing fails
  - Warning message should indicate output parsing issue

- How does system handle concurrent connection attempts (user runs `akon vpn on` twice)?
  - Second attempt should detect existing process and abort with clear message
  - Alternatively, prompt user to terminate existing connection first

- What happens when OpenConnect process hangs during connection?
  - System enforces 60-second connection timeout
  - After timeout, process is terminated and timeout error displayed

- How does system handle loss of connection after establishment?
  - System monitors OpenConnect process for unexpected termination
  - If process exits, user is notified with disconnection message

- What happens when user doesn't have permissions to create TUN device?
  - OpenConnect may fail with permissions error
  - System detects this specific error and suggests running with appropriate privileges or using alternative connection mode

- How does system handle OpenConnect requiring user interaction (e.g., certificate acceptance)?
  - System should detect interactive prompts in output
  - Display prompt to user and allow stdin pass-through for user response

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST spawn OpenConnect CLI as child process with appropriate arguments (server URL, --protocol=f5, username, non-interactive flags)

- **FR-002**: System MUST securely pass password to OpenConnect via stdin using `--passwd-on-stdin` flag (not command-line arguments)

- **FR-003**: System MUST capture and parse OpenConnect stdout/stderr with line-buffered parsing (<500ms latency per event) to extract connection state events; handle buffer overflow by logging warning and continuing with available data

- **FR-004**: System MUST detect successful connection by identifying "Connected" or "Configured as X.X.X.X" patterns in OpenConnect output

- **FR-005**: System MUST extract and display assigned IP address when connection is established

- **FR-006**: System MUST terminate OpenConnect process gracefully (SIGTERM) when user requests disconnection

- **FR-007**: System MUST force-kill OpenConnect process (SIGKILL) if graceful termination fails within 5 seconds

- **FR-008**: System MUST detect and report authentication failures by parsing OpenConnect error messages

- **FR-009**: System MUST implement connection timeout of 60 seconds, terminating process if connection is not established (rationale: 2x typical OTP validity window of 30s, accounting for network latency and server processing; configurable in future enhancement)

- **FR-010**: System MUST preserve existing credential management with full backward compatibility: maintain existing keyring key names, TOML config format, PIN+OTP generation logic unchanged (users do not need to re-setup credentials)

- **FR-011**: System MUST maintain output monitoring in background thread to avoid blocking main thread

- **FR-012**: System MUST track connection events in ordered list (authenticating, F5 session established, TUN configured, connected, disconnected, errors)

- **FR-013**: System MUST handle OpenConnect process unexpected termination and report disconnection to user

- **FR-014**: System MUST verify OpenConnect CLI is installed before attempting connection, providing helpful error if missing

- **FR-015**: System MUST log all OpenConnect output at debug level for troubleshooting purposes, excluding stdin content and authentication credentials per constitution security requirements

- **FR-016**: System MUST remove existing FFI bindings dependencies (bindgen, cc crate) from build process

- **FR-017**: System MUST remove C wrapper code (build.rs, wrapper.h, openconnect-internal.h, progress_shim.c) from codebase

- **FR-018**: System MUST delete existing `vpn/openconnect.rs` FFI module entirely (no deprecation period, immediate removal)

- **FR-019**: System MUST create new `vpn/cli_connector.rs` module implementing CLI-based connection

- **FR-020**: System MUST update `src/cli/vpn.rs` to use new CLI connector instead of FFI implementation

- **FR-021**: System MUST follow test-driven development (TDD) approach: write failing tests demonstrating expected OpenConnect CLI parsing behavior before implementing parser logic, ensuring red-green-refactor cycle for all `OutputParser` methods

- **FR-022**: System MUST log all connection state transitions (Authenticating→F5SessionEstablished→TunConfigured→Connected, or →Disconnected/Error) to systemd journal with structured metadata (timestamp, connection ID, server URL, assigned IP where applicable) for operational observability

- **FR-023**: System MUST verify process spawn capability during setup phase (not just OpenConnect presence), reporting helpful error if environment restricts child process creation (e.g., containerized or sandboxed environments)

- **FR-024**: System MUST maintain behavioral regression testing: preserve functional tests verifying VPN connection behavior (authentication, connection, disconnection, error handling); only delete tests specifically testing FFI binding internals; update test interfaces as needed to work with CLI implementation while maintaining same behavioral assertions

### Key Entities

- **CliConnector**: Process manager for OpenConnect CLI
  - Manages child process lifecycle (spawn, monitor, terminate)
  - Captures and parses stdout/stderr streams
  - Tracks connection state through parsed events
  - Provides methods for connect, disconnect, status queries

- **ConnectionEvent**: State transition events from OpenConnect output
  - Represents discrete states: Authenticating, F5SessionEstablished, TunConfigured, Connected, Disconnected, Error
  - Includes contextual data (IP address, error messages, device names)
  - Ordered chronologically in event list

- **OutputParser**: Parses OpenConnect CLI output into structured events
  - Uses pattern matching on output lines
  - Extracts data from output (IP addresses, device names, error messages)
  - Targets OpenConnect 9.x output format (latest stable)
  - Falls back to displaying raw output with warning when encountering unrecognized patterns
  - Version detection and multi-version adaptation deferred to future enhancement

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can establish VPN connection in under 30 seconds from command execution to "Connected" message

- **SC-002**: System detects connection completion within 2 seconds of OpenConnect establishing connection (based on output parsing)

- **SC-003**: All functional/behavioral tests continue passing (regression testing); only FFI-specific binding tests are deleted; tests with changed interfaces are updated to verify same behavior with CLI implementation; >90% coverage maintained for security-critical modules

- **SC-004**: Build time reduces by at least 50% due to removal of bindgen and C compilation steps

- **SC-005**: Codebase complexity reduces by at least 40% (measured by lines of code in VPN modules: baseline is `akon-core/src/vpn/openconnect.rs` + `akon-core/src/vpn/state.rs` + C wrapper files totaling ~800 lines; target is ~480 lines in new `cli_connector.rs`)

- **SC-006**: Zero unsafe Rust code in VPN connection modules (100% safe Rust)

- **SC-007**: Connection success rate achieves >95% with valid credentials to real F5 VPN servers

- **SC-008**: System successfully handles and reports at least 5 distinct error scenarios with appropriate user-facing messages

- **SC-009**: Disconnection completes within 5 seconds in 95% of cases (graceful termination)

- **SC-010**: System correctly parses connection state from latest stable OpenConnect CLI version (9.x); version detection and multi-version support deferred to future enhancement

## Dependencies & Assumptions *(mandatory)*

### Dependencies

- **External Dependency**: OpenConnect CLI must be installed on user's system (version 9.x recommended; latest stable)
  - User must install via package manager (apt, brew, etc.)
  - System validates presence before attempting connection
  - Must support F5 VPN protocol (--protocol=f5)
  - Initial implementation targets OpenConnect 9.x output format; backward compatibility with older versions deferred

- **Internal Dependencies**:
  - Existing credential management (keyring, PIN+OTP) remains unchanged
  - Configuration system (TOML config) remains unchanged
  - Error types and handling infrastructure (AkonError, VpnError) remains unchanged

### Assumptions

- **Assumption 1**: OpenConnect CLI output format is stable enough for parsing across minor versions
  - Rationale: OpenConnect has maintained relatively stable output format since v8.0
  - Risk: New versions may introduce breaking output changes
  - Mitigation: Implement flexible parsing with fallback to raw output display

- **Assumption 2**: Users running akon have permission to spawn child processes
  - Rationale: Standard user privilege, required for basic CLI functionality
  - Risk: Restricted environments (containers, sandboxes) may block process spawning
  - Mitigation: FR-023 requires verification during setup phase with helpful error messages; document system requirements clearly

- **Assumption 3**: OpenConnect CLI `--passwd-on-stdin` flag works consistently across versions
  - Rationale: This is a stable, documented feature since early OpenConnect versions
  - Risk: Very old or custom-built OpenConnect may not support this flag
  - Mitigation: Check OpenConnect version during setup phase

- **Assumption 4**: Credentials (PIN+OTP) are valid for the duration of connection attempt (60 seconds)
  - Rationale: OTP tokens typically have 30-60 second validity windows, sufficient for connection
  - Risk: Slow networks may cause timeout before connection completes
  - Mitigation: Configurable timeout in future enhancement

- **Assumption 5**: System can monitor process output without missing critical events
  - Rationale: Background thread monitoring with buffered reading provides reliable capture
  - Risk: High system load may introduce delays in event detection
  - Mitigation: Use non-blocking I/O and appropriate buffer sizes

- **Assumption 6**: TUN device creation failures are acceptable in non-root scenarios
  - Rationale: Current implementation acknowledges this limitation for debugging
  - Risk: Users may expect full routing functionality
  - Mitigation: Clear messaging about privilege requirements for full VPN routing

## Out of Scope

- Daemon mode implementation (deferred to future feature)
- Advanced VPN status tracking with bandwidth/latency metrics
- Automatic retry logic for transient connection failures
- Support for OpenConnect versions other than 9.x (version detection and multi-version support deferred)
- GUI or non-terminal interfaces
- Certificate pinning and advanced security configurations
- Split-tunnel routing configuration
- Multi-VPN connection management (multiple simultaneous connections)
- FFI implementation maintenance or backward compatibility with FFI-based versions

## Migration Strategy

### Immediate Replacement (Breaking Change)
1. Delete all FFI-related code in single atomic commit:
   - Remove `akon-core/build.rs` (bindgen configuration)
   - Remove C wrapper files: `wrapper.h`, `openconnect-internal.h`, `progress_shim.c`
   - Delete `akon-core/src/vpn/openconnect.rs` (FFI implementation)
   - Remove FFI dependencies from `Cargo.toml`: bindgen, cc crate
   - Delete only FFI-binding-specific tests (tests that directly test FFI internals/bindings)
2. Create new `akon-core/src/vpn/cli_connector.rs` module with CLI-based implementation
3. Update `src/cli/vpn.rs` to use CLI connector exclusively (no feature flags or fallback)
4. Preserve functional/behavioral tests; update test interfaces where needed to work with CLI implementation while verifying same behaviors (regression testing)
5. Implement additional unit and integration tests for new CLI-specific functionality
6. Update all documentation to reflect CLI-only architecture

### No Rollback Plan
- This is an intentional breaking change with no backward compatibility
- Users must upgrade to CLI-based implementation
- Previous git commits remain available for emergency reference only
- Clear communication in release notes about breaking change and migration requirements

## Technical Risks

### Risk 1: OpenConnect Output Parsing Fragility
**Severity**: Medium
**Impact**: Connection state detection may fail with new OpenConnect versions
**Mitigation**:
- Implement defensive parsing with fallback behaviors
- Test against multiple OpenConnect versions (8.0, 8.5, 9.0+)
- Provide raw output mode for debugging
- Monitor OpenConnect release notes for output format changes

### Risk 2: Process Management Complexity
**Severity**: Low
**Impact**: Edge cases in process lifecycle may cause resource leaks
**Mitigation**:
- Comprehensive integration tests for process termination scenarios
- Implement timeout guards on all process operations
- Use RAII pattern (Drop trait) for guaranteed cleanup

### Risk 3: Performance Regression
**Severity**: Low
**Impact**: CLI spawning overhead may add latency vs direct FFI
**Mitigation**:
- Benchmark connection establishment time
- Expected overhead is <500ms, acceptable for 30-second total connection time
- CLI approach is standard practice in similar tools (networkmanager, connman)

## Documentation Requirements

- Update README with new architecture explanation (CLI delegation, breaking change notice)
- Create troubleshooting guide for common OpenConnect CLI errors
- Document OpenConnect version requirement (9.x required; tested version and output format)
- Add architecture decision record (ADR) explaining rationale for CLI delegation and immediate FFI removal
- Update developer documentation with CLI connector API reference
- Create migration guide for users upgrading from FFI-based version (credential compatibility, breaking changes)

## Testing Requirements

### Test-Driven Development Requirements
- Tests MUST be written before implementation following red-green-refactor TDD cycle (per FR-021)
- Security-critical modules (credential passing via stdin, keyring integration) MUST achieve >90% test coverage per constitution Principle III
- All tests MUST be executable independently and in parallel where possible

### Regression Testing Strategy (per FR-024)
- Preserve all functional/behavioral tests (tests verifying VPN connection behavior, not FFI internals)
- Delete only FFI-binding-specific tests (tests that directly test FFI binding layer, C wrapper interfaces)
- Update test interfaces where necessary to work with CLI implementation while maintaining behavioral assertions
- All preserved tests MUST continue passing to ensure no functional regression

### Unit Tests
- Output parsing for all connection states (10+ test cases)
- IP address extraction from various output formats
- Error message parsing and classification
- Timeout handling logic

### Integration Tests
- Full connection lifecycle with mock OpenConnect process
- Graceful shutdown scenarios
- Abnormal termination handling
- Multiple concurrent connection attempts (error case)

### System Tests
- Process spawn capability verification in restricted environments (containers, sandboxes) per FR-023
- OpenConnect CLI presence detection and error messaging
- Credential flow from keyring through stdin to OpenConnect process

### Manual Testing Checklist
- Connection to real F5 VPN server with OpenConnect 9.x
- Connection with incorrect credentials (auth failure)
- Network interruption during connection
- OpenConnect CLI not installed (error handling)
- Connection timeout scenario (slow network)
- Graceful disconnection (akon vpn off)
- Interrupt-based disconnection (Ctrl+C)
- Credential backward compatibility (existing keyring entries work without re-setup)
