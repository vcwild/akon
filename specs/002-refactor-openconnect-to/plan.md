# Implementation Plan: OpenConnect CLI Delegation Refactor

**Branch**: `002-refactor-openconnect-to` | **Date**: 2025-11-03 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-refactor-openconnect-to/spec.md`

**Note**: This plan implements immediate FFI removal with CLI delegation for F5 VPN connections using OpenConnect 9.x.

## Summary

Replace FFI-based OpenConnect integration with direct CLI process management. Remove all FFI bindings, C wrappers, and build complexity. Implement CLI-based connector that spawns OpenConnect as child process, parses stdout/stderr for state events, and passes credentials via stdin. Breaking change with no backward compatibility for FFI code, but maintains credential/config compatibility. Targets OpenConnect 9.x with F5 protocol support. Implements TDD with regression testing strategy: preserve functional tests, delete only FFI-specific binding tests.

## Technical Context

**Language/Version**: Rust 1.70+ (stable channel)
**Primary Dependencies**:
- `tokio` for async runtime and process spawning
- `anyhow` for error handling
- Existing: `keyring`, `toml`, `serde` (preserved for backward compatibility)
- REMOVES: `bindgen`, `cc` crate (FFI build dependencies)
**Storage**:
- GNOME Keyring for credentials (unchanged)
- TOML config files `~/.config/akon/config.toml` (backward compatible)
**Testing**: `cargo test` with TDD approach
**Target Platform**: Linux (Ubuntu/Debian primary; systemd for logging)
**Project Type**: Single binary CLI application
**Performance Goals**:
- Connection establishment <30 seconds total
- Event parsing latency <500ms per event
- Build time reduction >50% (no C compilation)
**Constraints**:
- OpenConnect 9.x CLI must be installed (external dependency)
- F5 VPN protocol support required
- No unsafe Rust in VPN modules (SC-006)
- >90% test coverage for security-critical modules
**Scale/Scope**:
- Target LOC: ~480 lines in `cli_connector.rs` (40% reduction from ~800 baseline)
- 24 functional requirements
- 6 user stories (3 P1, 1 P2, 2 P3)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.0.0:

- [x] **Security-First**: Credentials stored only in GNOME Keyring ✓ (FR-010: backward compatibility maintained). Password passed via stdin (FR-002). Logging excludes credentials (FR-015). No plaintext secrets.
- [x] **Modular Architecture**: CliConnector, OutputParser, ConnectionEvent as independent modules ✓. Clear separation: process management, parsing, state tracking.
- [x] **Test-Driven Development**: FR-021 mandates TDD red-green-refactor cycle ✓. FR-024 requires regression testing ✓. Testing Requirements section specifies >90% coverage for security modules ✓.
- [x] **Observability**: FR-022 requires systemd journal logging of all state transitions ✓. FR-015 logs OpenConnect output at debug level (excluding credentials) ✓.
- [x] **CLI-First Interface**: All functionality via `akon vpn on/off/status` commands ✓. Human-readable + machine-parsable outputs (exit codes) ✓.

**Security-Critical Changes** (require extra scrutiny):
- [x] **OAuth token handling**: Not modified (out of scope)
- [x] **OTP generation algorithm**: Not modified (FR-010 preserves existing)
- [x] **Keyring operations**: Not modified (FR-010 backward compatibility)
- [x] **Password transmission to OpenConnect**: CHANGED - Now via stdin `--passwd-on-stdin` (FR-002) - constitution-compliant secure channel ✓
- [x] **Configuration parsing**: Not modified (FR-010 preserves TOML format)

## Project Structure

### Documentation (this feature)

```
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```
akon-core/
├── Cargo.toml           # Remove: bindgen, cc; Keep: tokio, anyhow, keyring, toml
├── src/
│   ├── lib.rs
│   ├── error.rs         # Preserved (AkonError, VpnError)
│   ├── types.rs         # Preserved
│   ├── auth/            # Preserved (credential management)
│   │   ├── mod.rs
│   │   ├── keyring.rs   # Unchanged
│   │   ├── totp.rs      # Unchanged
│   │   └── password.rs  # Unchanged
│   ├── config/          # Preserved (TOML parsing)
│   │   └── mod.rs
│   └── vpn/
│       ├── mod.rs
│       ├── cli_connector.rs    # NEW - CLI process manager
│       ├── output_parser.rs    # NEW - Parse OpenConnect output
│       ├── connection_event.rs # NEW - State event enum
│       └── state.rs            # Preserved/adapted
│
src/
├── main.rs
└── cli/
    ├── mod.rs
    ├── vpn.rs           # UPDATED - Use CliConnector
    ├── setup.rs         # Preserved
    └── get_password.rs  # Preserved

tests/
├── unit/
│   ├── output_parser_tests.rs  # NEW - TDD for parsing
│   ├── cli_connector_tests.rs  # NEW - Process lifecycle
│   └── [preserved functional tests]
├── integration/
│   ├── vpn_connection_tests.rs # UPDATED interfaces
│   ├── credential_flow_tests.rs # NEW - keyring→stdin→OpenConnect
│   └── [preserved behavioral tests]
└── system/
    └── process_spawn_tests.rs  # NEW - FR-023

# DELETED FILES (per Migration Strategy):
# - akon-core/build.rs
# - akon-core/wrapper.h
# - akon-core/openconnect-internal.h
# - akon-core/progress_shim.c
# - akon-core/src/vpn/openconnect.rs (FFI implementation)
# - tests/*_ffi_tests.rs (FFI binding tests only)
```

**Structure Decision**: Single Rust project with workspace layout. Core library (`akon-core`) contains VPN logic with new `cli_connector.rs` module. CLI binary (`src/`) delegates to core. Clear module boundaries: `auth/` (credentials), `config/` (TOML), `vpn/` (connection). Test structure mirrors source with unit/integration/system categories. FFI-related build files and C wrappers completely removed.

## Complexity Tracking

*Fill ONLY if Constitution Check has violations that must be justified*

**No constitution violations detected.** All checks passed. This refactoring actually **reduces** complexity:
- Removes C wrapper complexity (build.rs, bindgen, unsafe code)
- Simplifies to pure safe Rust with standard async/process APIs
- Reduces LOC by 40% (800 → 480 lines in VPN modules)
- Improves build time by >50% (no C compilation)
- Maintains modular architecture with clearer boundaries

---

## Phase 0: Research

**Objective**: Resolve technical unknowns and validate implementation patterns.

### Research Output

Generated `research.md` covering:

1. **Rust Process Spawning**: Selected `tokio::process::Command` with async I/O for non-blocking stdout/stderr monitoring
2. **OpenConnect 9.x Output Format**: Analyzed F5 protocol output patterns ("Connected tun0 as X.X.X.X", "Established connection", etc.)
3. **Line-Buffered Parsing**: Chose `BufReader::lines()` with `tokio::select!` for <500ms latency
4. **Process Termination**: SIGTERM→SIGKILL pattern with 5-second timeout
5. **Password Transmission**: `--passwd-on-stdin` via stdin.write_all() + immediate close
6. **Backward Compatibility**: Verified keyring keys and TOML structure unchanged
7. **Testing Strategy**: Three-tier categorization (delete FFI tests, preserve functional, update interfaces)
8. **Systemd Logging**: Selected `tracing` crate with journal backend
9. **Error Handling**: Pattern matching on stderr with fallback to raw display
10. **Performance Benchmarking**: `criterion` for regression testing

### Key Decisions

- **Primary Tool**: `tokio::process::Command` (non-blocking async I/O)
- **Target Version**: OpenConnect 9.12+ with F5 protocol
- **Event Parsing**: Line-based pattern matching (no structured OpenConnect output available)
- **Dependencies Added**: `tokio`, `tracing`, `criterion` (dev)
- **Dependencies Removed**: `bindgen`, `cc` (FFI build tools)

### No Blockers

All technical decisions resolved during clarification phase. No "NEEDS CLARIFICATION" items requiring additional research.

---

## Phase 1: Design & Contracts

**Objective**: Define data models, API contracts, and implementation blueprint.

### Design Artifacts Generated

#### 1. Data Model (`data-model.md`)

Defines three core entities and their relationships:

- **ConnectionEvent**: Enum representing VPN lifecycle state machine
  - 8 variants: ProcessStarted, Authenticating, F5SessionEstablished, TunConfigured, Connected, Disconnected, Error, UnknownOutput
  - State transition diagram: Start → Authenticating → F5SessionEstablished → TunConfigured → Connected
  - Consumed by CLI commands for user feedback

- **CliConnector**: Process lifecycle manager
  - Fields: `state`, `child_process`, `event_receiver`, `parser`, `config`
  - Key methods: `connect()`, `disconnect()`, `force_kill()`, `next_event()`
  - Internal state machine: Idle → Connecting → Authenticating → Established → Disconnecting
  - Uses Arc<Mutex<>> for thread-safe state sharing

- **OutputParser**: Pattern-based line parser
  - Regex patterns for F5 protocol output
  - Methods: `parse_line()`, `parse_error()`
  - Fallback to UnknownOutput for unrecognized lines
  - Extensible for future protocol support

**Relationships**: VpnConfig/Credentials → CliConnector → OutputParser → ConnectionEvent → CLI commands

#### 2. API Contracts (`contracts/`)

Three command-level contracts generated:

- **`vpn-on-command.md`**: Connection establishment flow
  - Interface: `async fn on_command(config: VpnConfig) -> Result<(), AkonError>`
  - 5-step implementation: Retrieve credentials → Create connector → Connect → Monitor events → Persist state
  - Timeout: 60 seconds with graceful cleanup
  - Error handling: Authentication, process spawn, timeout, network failures
  - Observability: All state transitions logged to systemd journal

- **`vpn-off-command.md`**: Graceful disconnection with SIGTERM→SIGKILL
  - Interface: `async fn off_command() -> Result<(), AkonError>`
  - 5-step implementation: Load state → Verify process → SIGTERM (5s timeout) → SIGKILL fallback → Cleanup state
  - Exit codes: 0 (success), 1 (no connection), 2 (error)
  - Edge cases: Stale state, unkillable processes, concurrent disconnects

- **`vpn-status-command.md`**: Connection status display
  - Interface: `fn status_command() -> Result<(), AkonError>`
  - 3-step implementation: Load state → Verify process → Display status
  - Output formats: Connected, Not connected, Stale state
  - Exit codes: 0 (connected), 1 (not connected), 2 (stale state)
  - Future: JSON output flag for machine-parsable status

#### 3. Quickstart Guide (`quickstart.md`)

7-phase TDD implementation guide:

1. **Setup** (15 min): Update Cargo.toml, remove FFI files, create modules
2. **Phase 1: Connection Events** (1 hour): TDD for ConnectionEvent enum
3. **Phase 2: Output Parser** (2 hours): Pattern matching implementation
4. **Phase 3: CLI Connector Skeleton** (3 hours): Basic structure and state management
5. **Phase 4: Process Management** (3 hours): Spawn, monitor stdout/stderr, credential passing
6. **Phase 5: Graceful Termination** (1 hour): SIGTERM→SIGKILL with timeout
7. **Phase 6: CLI Integration** (1 hour): Wire up commands to connector
8. **Phase 7: Testing & Validation** (1-2 hours): Full test suite, manual testing, benchmarks

**Total Estimated Time**: 8-12 hours

### Design Decisions

1. **Event-Driven Architecture**: Async event stream from OutputParser enables non-blocking feedback
2. **State Machine Pattern**: Type-safe ConnectionState enum prevents impossible states
3. **Fallback Parsing**: UnknownOutput variant ensures graceful degradation (FR-004)
4. **SIGTERM-first Termination**: Allows OpenConnect cleanup before force-kill
5. **State File Persistence**: Enables stateless `status` command without sudo

### Contract Guarantees

- **Type Safety**: All VPN states represented as Rust enums (compile-time verification)
- **Async I/O**: All process interactions use Tokio (no blocking operations)
- **Timeout Guarantees**: 60s connection timeout, 5s graceful shutdown timeout
- **Exit Code Consistency**: 0 (success), 1 (expected failure), 2 (unexpected error)
- **Observability**: All state transitions logged with structured metadata

### Dependencies Added

From research and design phases:

```toml
[dependencies]
tokio = { version = "1.35", features = ["process", "io-util", "time", "macros", "rt-multi-thread"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
regex = "1.10"
nix = "0.27"  # Signal handling (SIGTERM/SIGKILL)
chrono = "0.4"  # Connection duration tracking

[dev-dependencies]
criterion = "0.5"  # Performance benchmarking
tokio-test = "0.4"  # Async test helpers
```

### Agent Context Update Required

After Phase 1, update `.github/copilot-instructions.md`:

```markdown
## Active Technologies
- Rust 1.70+ (stable channel)
- Tokio 1.35+ (async runtime)
- OpenConnect 9.x (F5 protocol via CLI)

## Project Structure
- `akon-core/src/vpn/`: CLI connector, output parser, connection events
- `src/cli/vpn.rs`: VPN on/off/status commands
- `tests/`: Unit (output_parser, cli_connector), integration (credential_flow), system (process_spawn)

## Key Modules
- `CliConnector`: Manages OpenConnect process lifecycle
- `OutputParser`: Parses CLI output into ConnectionEvent
- `ConnectionEvent`: State machine enum for VPN lifecycle

## Commands
- `cargo test` - Run full test suite
- `cargo test --lib` - Unit tests only
- `cargo clippy` - Linter
- `cargo tarpaulin --out Html` - Coverage report
```

---

## Next Steps

**STOP HERE**: This plan is complete. Do NOT proceed to Phase 2 (tasks breakdown) in this command.

To continue implementation:
1. Run `/speckit.tasks` command (separate workflow) to generate `tasks.md`
2. Tasks will break down quickstart guide into granular development tasks
3. Follow TDD workflow: RED (failing test) → GREEN (minimal impl) → REFACTOR
