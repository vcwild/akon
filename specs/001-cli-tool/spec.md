# Feature Specification: OTP-Integrated VPN CLI with Secure Credential Management

**Feature Branch**: `001-cli-tool`
**Created**: 2025-10-08
**Status**: Draft
**Input**: User description: "CLI tool that handles the complexity of OTP authentication, that means that the project is able to use a one-time generated secret key and generate OTP by request and also use this generated token to automatically connect to libopenconnect. The project should communicate with openconnect via library code, not shell commands. It must handle stablishing the connection to the client and respond with clear feedback when a connection is successfull or not. It should be able to onboard the user when first time launching it. should store the user secret key safely and all sensitive information are stored in gnome-keyring, no passwords or sensitive data should leak in any case whatsoever."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - First-Time Setup with Secure Credential Storage (Priority: P1)

A new user installs the CLI tool and needs to configure their VPN credentials securely. The tool guides them through storing their VPN server details, username, 4-digit PIN, and OTP secret key in GNOME Keyring without ever exposing sensitive data in plaintext.

**Why this priority**: This is the foundation for all other functionality. Without secure credential storage, the tool cannot operate according to security principles. This delivers immediate value by ensuring user trust from first interaction.

**Independent Test**: Can be fully tested by running the setup command on a fresh system, verifying that credentials are stored in GNOME Keyring (using keyring inspection tools), and confirming no sensitive data appears in logs, config files, or environment variables.

**Acceptance Scenarios**:

1. **Given** a user runs the CLI for the first time, **When** they execute the setup command, **Then** the system prompts for VPN server URL, username, protocol, 4-digit PIN, and OTP secret key
2. **Given** the user provides valid credentials during setup, **When** the setup completes, **Then** all sensitive data (PIN, OTP secret, password if provided) is stored exclusively in GNOME Keyring with appropriate service names
3. **Given** the user provides an invalid PIN format, **When** setup validates the input, **Then** the system displays a clear error message explaining the expected format (exactly 4 numeric digits) and prompts for re-entry
4. **Given** the user provides an invalid OTP secret format, **When** setup validates the input, **Then** the system displays a clear error message explaining the expected format (Base32-encoded string) and prompts for re-entry
5. **Given** GNOME Keyring is not available on the system, **When** the user attempts setup, **Then** the system fails gracefully with a descriptive error message and instructions for installing the required keyring backend
6. **Given** setup completes successfully, **When** the user inspects the config file (`~/.config/akon/config.toml`), **Then** only non-sensitive data (VPN server, username, protocol) is present—no secrets (PIN and OTP secret are in keyring only)

---

### User Story 2 - Automatic VPN Connection with OTP Generation (Priority: P1)

A configured user wants to connect to their VPN using the stored credentials. The tool automatically generates a fresh OTP token from the secret key and establishes the VPN connection through the OpenConnect library, providing clear feedback on connection status.

**Why this priority**: This is the core value proposition—automated authentication without manual token generation. Essential for MVP as it delivers the primary user benefit.

**Independent Test**: Can be tested independently by configuring credentials via setup, then running the connect command and verifying: (1) OTP token is generated and used, (2) OpenConnect library is invoked (not shell commands), (3) Connection status is reported accurately, (4) No tokens appear in logs.

**Acceptance Scenarios**:

1. **Given** the user has completed setup with valid credentials, **When** they execute the connect command, **Then** the system retrieves the OTP secret from GNOME Keyring, generates a current TOTP token, and initiates the VPN connection
2. **Given** the VPN connection is successfully established, **When** the connection process completes, **Then** the system displays a success message with connection details (server, protocol, connected state) and exits with code 0
3. **Given** the VPN authentication fails (invalid OTP or credentials), **When** the connection attempt fails, **Then** the system displays a clear error message indicating authentication failure (without exposing the token value) and exits with code 1
4. **Given** the OpenConnect library encounters a network error, **When** the connection attempt fails, **Then** the system distinguishes between authentication errors and network errors in the user-facing message
5. **Given** the user wants to verify their connection, **When** they check the system's network interfaces, **Then** a VPN tunnel interface is present and routing traffic
6. **Given** the system generates an OTP token, **When** any logging occurs, **Then** the token value never appears in logs, stdout, or stderr—only sanitized events like "OTP generated successfully"

---

### User Story 3 - Manual Password Generation for External Use (Priority: P2)

A user needs to generate the complete VPN password (PIN + OTP) for use outside the CLI (e.g., manual browser login, troubleshooting, or integration with other tools). The tool provides a standalone command that outputs only the complete password to stdout for easy piping.

**Why this priority**: While not required for automated VPN connection, this enables debugging workflows and flexible integration with other tools. Supports the CLI-first principle by making functionality composable. Must match the exact password format used by auto-openconnect for compatibility.

**Independent Test**: Can be tested by running the get-password command after setup and verifying: (1) only the complete password (PIN + OTP, 10 characters) is printed to stdout, (2) any errors go to stderr, (3) the password is valid for VPN authentication, (4) exit codes are correct, (5) password format matches auto-openconnect output.

**Acceptance Scenarios**:

1. **Given** the user has stored PIN and OTP credentials, **When** they execute the get-password command, **Then** the system outputs only the current complete password (4-digit PIN + 6-digit TOTP = 10 characters) to stdout with no additional text
2. **Given** the get-password command succeeds, **When** the output is piped to another command, **Then** only the complete password value is passed (enabling `akon get-password | pbcopy` or similar)
3. **Given** the PIN or OTP secret is missing from GNOME Keyring, **When** the user runs get-password, **Then** an error message is written to stderr (not stdout) and the command exits with code 1
4. **Given** the generated password is used within its validity window (typically 30 seconds), **When** the password is submitted to the VPN server manually, **Then** it successfully authenticates
5. **Given** both akon and auto-openconnect are configured with the same PIN and OTP secret, **When** get-password is executed within the same 30-second TOTP window, **Then** both implementations produce identical 10-character passwords

---

### User Story 4 - VPN State Management (On/Off/Status) (Priority: P2)

A user wants to control their VPN connection lifecycle with simple commands: turn it on, turn it off, or check its status. The tool provides intuitive commands that manage the connection state and report back clearly.

**Why this priority**: Essential for user control and scripting, but dependent on the core connection functionality (P1). Enables automation and integration with system events (NetworkManager, systemd).

**Independent Test**: Can be tested by running each state command (on, off, status) independently and verifying: (1) on establishes connection, (2) off terminates cleanly, (3) status reports accurately, (4) all commands have appropriate exit codes.

**Acceptance Scenarios**:

1. **Given** the VPN is currently disconnected, **When** the user executes `akon vpn on`, **Then** the system initiates connection as in User Story 2
2. **Given** the VPN is currently connected, **When** the user executes `akon vpn off`, **Then** the system gracefully terminates the OpenConnect process and confirms disconnection
3. **Given** the VPN state is unknown, **When** the user executes `akon vpn status`, **Then** the system reports whether the VPN is connected or disconnected with connection details (server, uptime if connected)
4. **Given** the user runs `akon vpn on` when already connected, **When** the command detects an existing connection, **Then** the system displays a message indicating already connected and exits with code 0 (idempotent behavior)
5. **Given** the user runs `akon vpn off` when already disconnected, **When** the command detects no active connection, **Then** the system displays a message indicating already disconnected and exits with code 0

---

### User Story 5 - Automated Reconnection Monitoring (Priority: P3)

A user wants their VPN to automatically reconnect after network disruptions, system suspend/resume, or idle timeouts. The tool runs a background monitoring service that detects disconnections and re-establishes the connection without user intervention.

**Why this priority**: Enhances user experience but is not essential for MVP. Requires the core connection logic (P1) to be stable and well-tested first.

**Independent Test**: Can be tested by setting up the monitor service, simulating network events (disconnect WiFi, suspend system), and verifying automatic reconnection with proper logging of events.

**Acceptance Scenarios**:

1. **Given** the monitoring service is active and the VPN is connected, **When** a network interface goes down and comes back up, **Then** the monitor detects the disconnection and automatically reconnects within 30 seconds
2. **Given** the system suspends (sleep/hibernate), **When** the system resumes, **Then** the monitor re-establishes the VPN connection
3. **Given** the monitoring service encounters repeated connection failures, **When** the failure count exceeds a threshold (e.g., 3 failures in 5 minutes), **Then** the monitor logs an error and stops retrying (exponential backoff)
4. **Given** the user manually disconnects via `akon vpn off`, **When** the monitor detects this intentional disconnection, **Then** it does not attempt to reconnect (respects user intent)

---

### Edge Cases

- What happens when the GNOME Keyring is locked (user session not unlocked)?
  - System should detect locked keyring, prompt user to unlock it, and fail gracefully if unable to access credentials
- How does the system handle clock skew affecting TOTP token generation?
  - System should detect time synchronization issues (NTP) and warn the user if system time differs significantly from expected server time
- What happens when the OTP secret is corrupted or deleted from the keyring?
  - System should detect missing/invalid secret during connection attempt and prompt user to re-run setup
- How does the system handle multiple concurrent connection attempts?
  - System should detect existing connection process (via PID file or process check) and prevent duplicate connections
- What happens if the OpenConnect library version is incompatible?
  - System should validate library version on startup and display clear error if minimum required version is not met
- How does the system handle VPN server URL changes or maintenance windows?
  - System should allow config updates without requiring full setup re-run, and distinguish between temporary server errors and configuration problems

## Clarifications

### Session 2025-10-08

- Q: The specification currently references Python throughout (Python 3.13+, Python libraries, pytest, mypy). Given your requirement to implement in **Rust**, how should the VPN connection process run? → A: Single-process: Rust binary manages OpenConnect connection in-process using Rust FFI bindings to libopenconnect C library
- Q: For the VPN connection lifecycle in Rust, when the user runs `akon vpn on`, how should the process behave? → A: Hybrid approach: spawns a background process, but waits for a successful signal from this background process before giving back the terminal access to the user
- Q: For memory safety and performance in Rust, how should sensitive data (OTP secrets, TOTP tokens) be handled in memory? → A: Use `secrecy` crate: Type-safe secret wrapper (`Secret<T>`) that prevents accidental logging/display, with `ExposeSecret` for controlled access
- Q: For async runtime in Rust (needed for network I/O with OpenConnect FFI and monitoring network events), which approach should be used? → A: Synchronous only: Avoid async complexity, use blocking I/O with threads for concurrent operations (monitoring service in separate thread)
- Q: For error handling strategy in Rust, how should errors be propagated and presented to users? → A: `thiserror` + `anyhow`: Use `thiserror` for library error types with context, `anyhow` for application-level error chaining with `.context()`
- Q: Should the system support credential export/import for migration to new systems or restore after keyring corruption? → A: No export/import: User must run `akon setup` again on new systems (secrets never leave keyring)
- Q: What should happen if the user runs `akon setup` multiple times with different credentials? → A: Prompt for confirmation: Detect existing credentials and ask "Overwrite existing setup? (y/N)"
- Q: What should be the specific exponential backoff strategy for reconnection attempts? → A: Conservative: Initial 5s, max 5 minutes, factor 2.0 (5s → 10s → 20s → 40s → 80s → 160s → 300s cap)
- Q: How should the system handle VPN server rate limiting (when the server blocks repeated connection attempts)? → A: Respect HTTP 429/rate limit errors: Detect rate limiting responses, log error, stop retries until user manually reconnects
- Q: Should the MVP support multiple VPN profiles or single profile only? → A: Single profile only: One config file, one set of credentials, simplest implementation for MVP

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a first-time setup command that collects VPN server URL, username, protocol (ssl/nc), 4-digit PIN, and OTP secret key through secure prompts (no command-line arguments for secrets); if credentials already exist, system MUST detect them and prompt "Overwrite existing setup? (y/N)" before proceeding
- **FR-002**: System MUST store all sensitive data (PIN, OTP secret key, passwords) exclusively in GNOME Keyring using appropriate service identifiers (e.g., `akon-vpn-pin`, `akon-vpn-otp`); credentials MUST NOT be exportable—users must run setup again on new systems to maintain security
- **FR-003**: System MUST store non-sensitive configuration (VPN server, username, protocol) in a TOML file at `~/.config/akon/config.toml` with clear separation from secrets; MVP supports single profile only (one config file, one set of credentials), multi-profile support deferred to future iterations
- **FR-004**: System MUST generate TOTP tokens using **the exact same algorithm as auto-openconnect** (custom HMAC-SHA1 implementation following RFC 6238 and RFC 2104) with standard 30-second time steps to ensure cross-compatibility; tokens MUST match those generated by auto-openconnect's `lib.py::generate_otp()` function for the same secret and time window
- **FR-004a**: System MUST implement custom Base32 decoding logic that matches auto-openconnect's behavior: (1) remove all whitespace characters from input, (2) apply padding to 8-character boundaries using `=` characters following the formula: `padding_length = (8 - (len(input) % 8)) % 8`
- **FR-004b**: System MUST implement HOTP counter calculation as `current_unix_timestamp / 30` (integer division) to match Python's behavior exactly
- **FR-004c**: System MUST implement custom HMAC-SHA1 following RFC 2104 with 64-byte block size, matching auto-openconnect's implementation (ipad=0x36, opad=0x5C, SHA1 hash function)
- **FR-005**: System MUST validate 4-digit PIN format during setup (exactly 4 numeric characters, no letters or special characters); System MUST validate OTP secret key format during setup (Base32-encoded string, typically 16-32 characters)
- **FR-006**: System MUST communicate with OpenConnect through Rust FFI bindings to libopenconnect C library in-process, not by spawning shell commands or external processes
- **FR-007**: System MUST pass complete passwords (PIN + OTP) to OpenConnect via secure in-memory channels using FFI callbacks, wrapped in `secrecy::Secret<T>` to prevent accidental exposure; password format MUST be exactly: 4-digit PIN concatenated with 6-digit OTP (10 characters total)
- **FR-008**: System MUST provide clear connection feedback with three states: connecting, connected (with connection details), failed (with error category)
- **FR-009**: System MUST distinguish between authentication failures, network errors, and configuration errors in user-facing messages using structured error types (`thiserror` for library errors, `anyhow` with context for application errors)
- **FR-010**: System MUST return appropriate exit codes: 0 for success, 1 for authentication/network failures, 2 for configuration errors
- **FR-011**: System MUST provide a `get-password` command that outputs only the current complete password (4-digit PIN + 6-digit TOTP = 10 characters) to stdout (machine-parsable), with errors to stderr; output format MUST match auto-openconnect's `get-password` command exactly for cross-compatibility
- **FR-012**: System MUST provide VPN state management commands: `on` (connect—spawns background daemon, blocks until connection established or fails, then returns terminal control), `off` (disconnect), `status` (report state)
- **FR-013**: System MUST gracefully handle missing GNOME Keyring backend with actionable error messages and setup instructions
- **FR-014**: System MUST prevent credential leakage in logs by sanitizing all logging output (never log PINs, OTP tokens, passwords, or secret keys); all sensitive values MUST be wrapped in `secrecy::Secret<T>` which prevents accidental Debug/Display formatting
- **FR-015**: System MUST log security-relevant events to systemd journal (keyring access, OTP generation requests, connection attempts, authentication results)
- **FR-016**: System MUST validate that GNOME Keyring is accessible before attempting credential operations
- **FR-017**: System MUST support `--config` flag to override default config file location for scripting and testing
- **FR-018**: System MUST implement idempotent connect/disconnect operations (connecting when already connected returns success, disconnecting when disconnected returns success)
- **FR-019**: System MUST provide a monitoring service (running in separate OS thread, not async) that detects network state changes and automatically reconnects when appropriate
- **FR-020**: System MUST respect user-initiated disconnections and not auto-reconnect after explicit `vpn off` commands
- **FR-021**: System MUST implement exponential backoff for reconnection attempts with parameters: initial delay 5 seconds, maximum delay 5 minutes (300 seconds), backoff factor 2.0, producing sequence: 5s → 10s → 20s → 40s → 80s → 160s → 300s (capped)
- **FR-022**: System MUST validate OpenConnect library availability and version compatibility during initialization
- **FR-023**: System MUST detect VPN server rate limiting (HTTP 429 or equivalent rate limit responses from OpenConnect library), log the rate limit error with timestamp, and stop automatic reconnection attempts until user manually initiates reconnection via `akon vpn on`

### Key Entities

- **VPN Configuration**: Represents the non-sensitive connection parameters (server URL, username, protocol, optional port). Stored in TOML format with clear schema. Validated on load for required fields and format. Rust struct with `serde` derives for deserialization.
- **PIN**: A 4-digit numeric code used as the first part of VPN authentication. Stored exclusively in GNOME Keyring with service name `akon-vpn-pin`. In-memory representation wrapped in `secrecy::Secret<String>`. Combined with OTP to form the complete password (PIN + OTP = 10 characters). Validated during setup to be exactly 4 digits.
- **OTP Secret**: The Base32-encoded seed used for TOTP generation. Stored exclusively in GNOME Keyring with service name `akon-vpn-otp`. In-memory representation wrapped in `secrecy::Secret<String>` to prevent accidental exposure. Never transmitted or logged. Validated for format and length during setup. **Must use algorithm compatible with auto-openconnect's lib.py implementation.**
- **Complete Password**: The concatenation of PIN + OTP (10 characters total: 4-digit PIN + 6-digit OTP). Generated on-demand by retrieving PIN from keyring and generating TOTP token. Passed to OpenConnect for authentication. Wrapped in `secrecy::Secret<String>`. Never stored, only computed when needed. Format MUST match auto-openconnect exactly.
- **Connection State**: Represents the current VPN connection status (disconnected, connecting, connected, error). Includes metadata like connection start time, server endpoint, and last error. Observable through status command and monitoring service. Shared between main thread and monitoring thread via `Arc<Mutex<ConnectionState>>`.
- **Keyring Entry**: A secure credential stored in GNOME Keyring. Includes service name, username (account identifier), and secret value. Accessed only through the keyring API, never directly read or written to disk.

### Rust-Specific Architecture

**Process Model**:

- Main CLI process: Handles user commands, setup, credential operations
- Background daemon process: Spawned by `vpn on`, manages OpenConnect FFI connection in-process, communicates back to parent via IPC (Unix socket or signals)
- Parent blocks on connection establishment, then releases terminal control while daemon continues running
- Monitoring thread: Separate OS thread within daemon process for network state observation

**Error Handling**:

- Library modules (auth, keyring, vpn connection) define custom error enums using `thiserror` with structured variants (e.g., `KeyringError::Locked`, `VpnError::AuthenticationFailed`)
- Application CLI code uses `anyhow::Result<T>` with `.context()` to add actionable error messages
- All errors map to appropriate exit codes: 0 (success), 1 (auth/network failure), 2 (configuration error)

**Memory Safety**:

- OTP secrets: `secrecy::Secret<String>` prevents accidental Debug/Display output
- TOTP tokens: `secrecy::Secret<String>` for generated codes, exposed only when passing to OpenConnect FFI
- FFI boundaries: Use `std::ffi::CString` for C string conversion, validate pointer lifetimes carefully
- No unsafe code in security-critical paths (OTP generation, credential handling) unless required for FFI, and audited thoroughly

**Concurrency Model**:

- No async runtime dependency (Tokio/async-std)
- Monitoring service runs in dedicated OS thread (`std::thread::spawn`)
- Shared state protected by `std::sync::Mutex` or `std::sync::RwLock`
- Inter-process communication between CLI and daemon via Unix domain sockets or signal handlers

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can complete first-time setup (credential storage including PIN and OTP secret) in under 3 minutes with zero credential exposure in config files or logs
- **SC-002**: Users can establish VPN connection in under 10 seconds from command execution to connected state (excluding network latency)
- **SC-003**: 100% of sensitive data (PINs, OTP secrets, passwords) is stored in GNOME Keyring—zero instances of plaintext secrets in filesystem, environment variables, or logs
- **SC-004**: System provides actionable error messages for 100% of failure modes (authentication, network, configuration, keyring access)
- **SC-005**: Complete passwords (PIN + OTP) generated by akon MUST match passwords generated by auto-openconnect's `password_generator.py` for the same PIN, secret, and time window (validated via integration tests comparing outputs from both implementations within same 30-second TOTP window)
- **SC-006**: Automated reconnection succeeds within 30 seconds of network restoration in 95%+ of cases
- **SC-007**: Users can integrate the `get-password` command with external tools (piping, scripting) without parsing complexity; output format matches auto-openconnect exactly (10 characters, no separators)
- **SC-008**: System maintains >90% code coverage for security-critical modules (PIN storage, OTP generation, keyring operations, credential handling)

## Assumptions

- **Platform**: Primary target is Linux systems with GNOME desktop environment and GNOME Keyring installed. macOS and Windows support deferred to future iterations.
- **Rust Version**: Requires Rust 1.70+ (stable channel) for modern language features and robust ecosystem support.
- **OpenConnect Library**: Assumes libopenconnect C library is installed and accessible via Rust FFI bindings (either through existing crates or custom bindings using bindgen).
- **Time Synchronization**: Assumes system clock is synchronized via NTP or equivalent (TOTP requires accurate time within ±30 seconds).
- **Keyring Backend**: Assumes GNOME Keyring is the primary secure storage backend accessed through Rust bindings to libsecret. Other keyring backends (KWallet, macOS Keychain) are not explicitly supported in MVP.
- **User Permissions**: Assumes user has necessary permissions to install system packages (OpenConnect, keyring dependencies) and configure network interfaces.
- **VPN Protocol**: Assumes VPN server supports OpenConnect protocol (Cisco AnyConnect-compatible) with OTP authentication methods.
- **OTP Standard**: Assumes TOTP (RFC 6238) with standard parameters: 30-second time step, 6-8 digit codes, HMAC-SHA1/SHA256.
- **Network Environment**: Assumes user has network access to VPN server and is not behind a captive portal or firewall blocking VPN protocols during initial connection.
- **Monitoring Integration**: Assumes systemd is available for journal logging and service management (NetworkManager dispatchers for event detection).

## Dependencies

- **External**: OpenConnect (libopenconnect) C library for VPN protocol implementation
- **External**: GNOME Keyring (libsecret) for secure credential storage
- **Rust Crates**:
  - `secrecy` (type-safe secret handling preventing accidental exposure)
  - `thiserror` (structured error types for library code)
  - `anyhow` (ergonomic error handling with context for application code)
  - `libsecret` or `secret-service` (GNOME Keyring/libsecret bindings)
  - `toml` or `serde_toml` (TOML configuration parsing)
  - `totp-lite` or similar (TOTP token generation, RFC 6238)
  - `clap` (CLI argument parsing)
  - `tracing` or `log` + `systemd-journal-logger` (structured logging to systemd journal)
  - FFI bindings to libopenconnect (via `openconnect-sys` if available, or custom bindgen-generated bindings)
- **System**: NetworkManager (for monitoring network events), systemd (for logging and optional service management), D-Bus (for keyring and NetworkManager communication)
- **Build**: `cargo` (Rust build system), `bindgen` (if generating custom FFI bindings), `cargo-tarpaulin` or `cargo-llvm-cov` (code coverage)

## Constraints

- **Security**: No credential storage outside GNOME Keyring—this is non-negotiable per project constitution
- **Library Communication**: Must use OpenConnect library API, not shell command spawning—required for proper error handling and secure credential passing
- **Logging**: Must never log sensitive values (tokens, passwords, secret keys)—all logging must be security-audited
- **CLI Interface**: All functionality must be CLI-accessible for automation and scripting—no GUI-only features
- **Testing**: Security-critical modules must achieve >90% code coverage per constitution's TDD principle; use `cargo test` with `cargo-tarpaulin` or `cargo-llvm-cov`
- **Modularity**: Must decompose into independent modules (auth, config, connection, monitoring) per constitution's modular architecture principle; leverage Rust's module system and trait-based abstractions
- **Concurrency**: Use OS threads (not async runtime) for concurrent operations to maintain simplicity and robustness in CLI context
- **Memory Safety**: All sensitive data wrapped in `secrecy::Secret<T>` to leverage Rust's type system for preventing credential leakage
- **Platform**: Linux-first approach—other platforms are not in scope for MVP
