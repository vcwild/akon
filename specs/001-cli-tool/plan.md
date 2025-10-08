# Implementation Plan: OTP-Integrated VPN CLI with Secure Credential Management

**Branch**: `001-cli-tool` | **Date**: 2025-10-08 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-cli-tool/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Build a Rust-based CLI tool that automates VPN connection via OpenConnect with OTP authentication. The tool securely stores credentials in GNOME Keyring, generates TOTP tokens on demand, and manages VPN connection lifecycle. Focus for this iteration: **setup command** (credential storage), **vpn on command** (connect with OTP), and **vpn off command** (disconnect).

## Technical Context

**Language/Version**: Rust 1.70+ (stable channel)
**Primary Dependencies**:

- `clap` (CLI parsing)
- `secrecy` (type-safe secret handling)
- `thiserror` + `anyhow` (error handling)
- `serde` + `toml` (configuration)
- `totp-lite` or equivalent (TOTP generation)
- `secret-service` or `libsecret` (GNOME Keyring bindings)
- `tracing` + `systemd-journal-logger` (logging)
- Custom FFI bindings to libopenconnect (via `bindgen`)

**Storage**:

- Secrets: GNOME Keyring (via D-Bus/libsecret)
- Configuration: `~/.config/akon/config.toml` (non-sensitive only)
- State: PID file and Unix socket for daemon communication

**Testing**: `cargo test` with `cargo-tarpaulin` or `cargo-llvm-cov` for coverage
**Target Platform**: Linux (GNOME desktop environment)
**Project Type**: Single binary CLI application with daemon mode
**Performance Goals**:

- VPN connection establishment: <10 seconds (excluding network latency)
- Setup completion: <3 minutes
- TOTP generation: <100ms

**Constraints**:

- Zero sensitive data in logs, config files, or environment
- >90% code coverage for security-critical modules
- No async runtime (use OS threads)
- All secrets wrapped in `secrecy::Secret<T>`
- FFI to libopenconnect (no shell command spawning)

**Scale/Scope**: Single-user CLI tool, ~5-10 commands, 3-5K LOC

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.0.0:

- [x] **Security-First**: All credentials stored only in GNOME Keyring via libsecret bindings. Secrets wrapped in `secrecy::Secret<T>`. No plaintext in config/logs. FR-002, FR-007, FR-014 enforce this.
- [x] **Modular Architecture**: Decomposed into independent modules: `auth` (keyring + OTP), `config` (TOML parsing), `vpn` (OpenConnect FFI), `cli` (command routing), `daemon` (background process). Each testable in isolation.
- [x] **Test-Driven Development**: TDD workflow mandated. Security modules (auth, OTP) require >90% coverage per constraint. Unit tests for OTP/config, integration tests for keyring/FFI, system tests for daemon lifecycle.
- [x] **Observability**: All state transitions logged to systemd journal via `tracing` + `systemd-journal-logger`. FR-015 requires security event logging. Sensitive values excluded per FR-014.
- [x] **CLI-First Interface**: Primary interface is CLI (`akon setup`, `akon vpn on/off`, `akon get-password`). Machine-parsable output (exit codes, stdout/stderr separation). FR-011, FR-012 define composability.

**Security-Critical Changes** (require extra scrutiny):

- [x] OTP secret storage (GNOME Keyring only, service name `akon-vpn-otp`)
- [x] OTP generation algorithm (TOTP RFC 6238, HMAC-SHA1/SHA256, using `totp-lite` crate)
- [x] Keyring operations (libsecret bindings via `secret-service` crate, D-Bus communication)
- [x] Password transmission to OpenConnect (FFI callbacks with `secrecy::Secret<String>`, no shell)
- [x] Configuration parsing (TOML for public data only, secrets never in config files)

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

```text
akon/                          # Repository root
├── Cargo.toml                 # Workspace manifest
├── Cargo.lock
├── .cargo/
│   └── config.toml            # Build configuration
├── src/                       # Binary crate (CLI entry point)
│   ├── main.rs                # CLI command router
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── setup.rs           # `akon setup` command
│   │   ├── vpn.rs             # `akon vpn on/off/status` commands
│   │   └── get_password.rs    # `akon get-password` command
│   └── daemon/
│       ├── mod.rs
│       ├── process.rs         # Background daemon process management
│       └── ipc.rs             # Unix socket/signal IPC
├── akon-core/                 # Library crate (shared logic)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── auth/              # Authentication module
│   │   │   ├── mod.rs
│   │   │   ├── keyring.rs     # GNOME Keyring operations
│   │   │   └── totp.rs        # TOTP generation (RFC 6238)
│   │   ├── config/            # Configuration module
│   │   │   ├── mod.rs
│   │   │   └── toml_config.rs # TOML parsing/validation
│   │   ├── vpn/               # VPN connection module
│   │   │   ├── mod.rs
│   │   │   ├── openconnect.rs # FFI to libopenconnect
│   │   │   └── state.rs       # Connection state management
│   │   ├── error.rs           # Error types (thiserror)
│   │   └── types.rs           # Shared types (Secret<T> wrappers)
│   └── build.rs               # bindgen for libopenconnect FFI
├── tests/                     # Integration tests
│   ├── integration/
│   │   ├── keyring_tests.rs   # GNOME Keyring integration
│   │   ├── config_tests.rs    # File I/O, TOML parsing
│   │   └── daemon_tests.rs    # Process spawning, IPC
│   └── fixtures/
│       └── test_configs/      # Mock TOML files
└── target/                    # Build artifacts (gitignored)
```

**Structure Decision**: Single Rust workspace with binary crate (`src/`) and library crate (`akon-core/`). Binary handles CLI routing and daemon process management. Library provides testable modules for auth, config, and VPN logic. This separation enables unit testing of core logic without CLI context and facilitates future daemon-only mode.

## Complexity Tracking

**No constitution violations detected.** All principles satisfied:

- Security-First: Enforced via `secrecy` crate and GNOME Keyring
- Modular: Clean separation of auth/config/vpn/cli modules
- TDD: Mandated with >90% coverage for security modules
- Observable: systemd journal logging with tracing
- CLI-First: Primary interface with composable commands

---

## Phase 0: Research & Technology Decisions ✅

**Status**: Complete
**Output**: [`research.md`](./research.md)

### Key Decisions

| Component | Technology | Status |
|-----------|-----------|--------|
| VPN Connection | FFI to libopenconnect via bindgen | ✅ Researched |
| Credential Storage | `secret-service` (GNOME Keyring) | ✅ Researched |
| OTP Generation | `totp-lite` crate | ✅ Researched |
| Error Handling | `thiserror` + `anyhow` | ✅ Researched |
| CLI Framework | `clap` v4 derive | ✅ Researched |
| Logging | `tracing` + `tracing-journald` | ✅ Researched |
| Daemon Process | `daemonize` + Unix sockets | ✅ Researched |
| Configuration | `serde` + `toml` + `directories` | ✅ Researched |

**All NEEDS CLARIFICATION items resolved.**

---

## Phase 1: Design & Contracts ✅

**Status**: Complete
**Outputs**:

- [`data-model.md`](./data-model.md) - Core data structures and entity relationships
- [`contracts/setup-command.md`](./contracts/setup-command.md) - Setup command specification
- [`contracts/vpn-on-command.md`](./contracts/vpn-on-command.md) - VPN connect command specification
- [`contracts/vpn-off-command.md`](./contracts/vpn-off-command.md) - VPN disconnect command specification
- [`quickstart.md`](./quickstart.md) - User-facing quick start guide
- `.github/copilot-instructions.md` - Updated agent context with Rust tech stack

### Data Model Summary

**Core Entities**:

1. `VpnConfig` - Non-sensitive configuration (TOML file)
2. `OtpSecret` - Base32-encoded TOTP seed (GNOME Keyring, wrapped in `secrecy::Secret<T>`)
3. `TotpToken` - Generated one-time password (memory only, auto-zeroized)
4. `ConnectionState` - VPN status with metadata (shared via IPC)
5. `KeyringEntry` - Keyring operation metadata
6. `IpcMessage` - Daemon communication protocol (JSON over Unix socket)

### CLI Commands (This Iteration)

| Command | Priority | Status | Contract |
|---------|----------|--------|----------|
| `akon setup` | P1 | Specified | [setup-command.md](./contracts/setup-command.md) |
| `akon vpn on` | P1 | Specified | [vpn-on-command.md](./contracts/vpn-on-command.md) |
| `akon vpn off` | P1 | Specified | [vpn-off-command.md](./contracts/vpn-off-command.md) |

**Deferred to Future Iterations**:

- `akon vpn status` (P2) - User Story 4
- `akon get-password` (P2) - User Story 3
- Auto-reconnect monitoring (P3) - User Story 5

### Constitution Re-Check (Post-Design)

- [x] **Security-First**: All secrets wrapped in `secrecy::Secret<T>`, GNOME Keyring storage, no plaintext exposure
- [x] **Modular Architecture**: Clean module boundaries (`auth`, `config`, `vpn`, `cli`, `daemon`)
- [x] **Test-Driven Development**: Test scenarios defined for each command, >90% coverage target for security modules
- [x] **Observability**: Structured logging to systemd journal with tracing, security events logged, no secret values
- [x] **CLI-First Interface**: All functionality accessible via CLI, composable outputs (exit codes, stdout/stderr separation)

**No violations introduced during design phase.**

---

## Phase 2: Task Decomposition

**Status**: Not started (run `/speckit.tasks` to generate tasks.md)

**Next Steps**:

1. Run `/speckit.tasks` to decompose implementation into atomic tasks
2. Tasks will be prioritized by dependency order:
   - Foundation: Error types, config parsing, keyring operations
   - Core: OTP generation, OpenConnect FFI bindings
   - Commands: Setup → VPN On → VPN Off
   - Testing: Unit → Integration → System tests

---

## Implementation Roadmap

### MVP Scope (This Iteration)

Focus on **secure credential storage** and **basic VPN lifecycle** (setup, connect, disconnect):

**Week 1: Foundation**

- [ ] Project structure (Cargo workspace, crates)
- [ ] Error types (`thiserror` enums)
- [ ] Configuration module (TOML parsing, validation)
- [ ] Logging setup (`tracing` + journald)

**Week 2: Security & Auth**

- [ ] Keyring integration (`secret-service` crate)
- [ ] OTP secret storage/retrieval
- [ ] TOTP generation (`totp-lite`)
- [ ] Unit tests (>90% coverage)

**Week 3: VPN Connection**

- [ ] OpenConnect FFI bindings (`bindgen`)
- [ ] VPN connection module (establish/teardown)
- [ ] Safe FFI wrappers
- [ ] Integration tests (mock OpenConnect)

**Week 4: CLI & Daemon**

- [ ] CLI framework (`clap` commands)
- [ ] Setup command implementation
- [ ] Daemon process management
- [ ] Unix socket IPC
- [ ] VPN on/off commands

**Week 5: Testing & Polish**

- [ ] System tests (end-to-end flows)
- [ ] Error handling polish
- [ ] Documentation
- [ ] Code coverage validation (>90% for auth module)

### Future Iterations

- **Iteration 2**: Status command, get-password command (P2 features)
- **Iteration 3**: Auto-reconnect monitoring service (P3 feature)
- **Iteration 4**: Config management, multiple profiles

---

## Artifacts Generated

✅ `/specs/001-cli-tool/plan.md` - This file
✅ `/specs/001-cli-tool/research.md` - Technology research and decisions
✅ `/specs/001-cli-tool/data-model.md` - Core data structures and relationships
✅ `/specs/001-cli-tool/contracts/setup-command.md` - Setup command specification
✅ `/specs/001-cli-tool/contracts/vpn-on-command.md` - VPN connect specification
✅ `/specs/001-cli-tool/contracts/vpn-off-command.md` - VPN disconnect specification
✅ `/specs/001-cli-tool/quickstart.md` - User quick start guide
✅ `.github/copilot-instructions.md` - Updated agent context

**Next Command**: `/speckit.tasks` to generate task decomposition

---

## Summary

**Branch**: `001-cli-tool`
**Spec**: [spec.md](./spec.md)
**Plan Status**: Phase 0 & Phase 1 Complete ✅

**Key Achievements**:

- All technology choices finalized (Rust + specific crates)
- Data model designed with security-first approach (`secrecy` crate)
- CLI contracts specified for setup, connect, disconnect commands
- Process architecture defined (daemon model with Unix socket IPC)
- Constitution compliance verified (no violations)
- Agent context updated with Rust tech stack

**Ready to Proceed**:

Run `/speckit.tasks` to generate implementation tasks for the MVP iteration (setup + vpn on/off commands).
