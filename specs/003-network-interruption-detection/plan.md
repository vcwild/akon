# Implementation Plan: Network Interruption Detection and Automatic Reconnection

**Branch**: `003-network-interruption-detection` | **Date**: 2025-11-04 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/003-network-interruption-detection/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Implement automatic VPN reconnection when network interruptions occur (WiFi changes, suspend/resume, network switches). The system will detect stale OpenConnect connections through network event monitoring and periodic HTTP/HTTPS health checks (60s interval), clean up orphaned processes, wait for network stability (health check endpoint reachability), and automatically reconnect using exponential backoff (5s, 10s, 20s, 40s up to 60s max, 5 max attempts). Reconnection state will be tracked with a new `Reconnecting { attempt, next_retry_at }` state, visible via `akon vpn status` and logged to system journal.

## Technical Context

**Language/Version**: Rust 1.70+ (stable channel, edition 2021)

**Primary Dependencies**:

- tokio (async runtime, process management, timers)
- dbus/zbus (NEEDS CLARIFICATION: NetworkManager D-Bus integration)
- reqwest/hyper (NEEDS CLARIFICATION: HTTP/HTTPS health check client)
- tracing/tracing-journald (structured logging to systemd journal)
- nix/libc (process signal handling)

**Storage**: State file for reconnection tracking (TOML or JSON), existing GNOME Keyring for credentials

**Testing**: cargo test, integration tests with mock D-Bus and HTTP endpoints

**Target Platform**: Linux with NetworkManager and systemd (GNOME Keyring environment)

**Project Type**: Single binary CLI application with workspace structure (akon + akon-core library)

**Performance Goals**:

- Detect network interruption within 5 seconds
- Reconnect within 10 seconds of network stability
- Health check complete within 5 seconds
- Minimal CPU/memory overhead when idle

**Constraints**:

- Must not block CLI commands during reconnection
- Health checks must not interfere with VPN traffic
- State must survive process crashes
- Exponential backoff must prevent server overload

**Scale/Scope**:

- Single-user VPN client
- Track 1 active connection at a time
- Support configurable retry policies
- ~5-10 new modules/files

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.0.0:

- [x] **Security-First**: Are credentials stored only in GNOME Keyring? No plaintext secrets in code/config/logs?
  - ✅ Feature uses existing keyring integration for OTP retrieval during reconnection
  - ✅ No new credential storage mechanisms introduced
  - ✅ Health check endpoint URL is non-sensitive configuration (server address)
  - ✅ Logging will sanitize any sensitive reconnection data
- [x] **Modular Architecture**: Is functionality decomposed into independent modules with clear boundaries?
  - ✅ Network event monitoring module (D-Bus integration)
  - ✅ Health check module (HTTP client)
  - ✅ Reconnection state machine module
  - ✅ Exponential backoff scheduler module
  - ✅ Each module independently testable
- [x] **Test-Driven Development**: Are tests written before implementation? Security modules >90% coverage?
  - ✅ Health check module tests (mock HTTP responses)
  - ✅ Exponential backoff algorithm tests (deterministic)
  - ✅ State machine transition tests
  - ✅ Network event detection tests (mock D-Bus)
  - ✅ Integration tests for full reconnection flow
- [x] **Observability**: Are all state changes logged to systemd journal? No secrets in logs?
  - ✅ All reconnection state transitions logged (Disconnected → Reconnecting → Connected/Error)
  - ✅ Health check results logged (success/failure counts, endpoint reachability)
  - ✅ Exponential backoff decisions logged (attempt number, next retry time)
  - ✅ Network event detection logged (WiFi change, suspend/resume)
  - ✅ No OTP tokens or credentials in logs
- [x] **CLI-First Interface**: Is functionality accessible via CLI with composable outputs?
  - ✅ `akon vpn status` shows reconnection state with attempt details
  - ✅ `akon vpn cleanup` for manual process cleanup (new command)
  - ✅ Exit codes indicate reconnection status
  - ✅ Machine-parsable output for scripts

**Security-Critical Changes** (require extra scrutiny):

- [x] OAuth token handling - **NOT MODIFIED** (uses existing keyring retrieval)
- [x] OTP generation algorithm - **NOT MODIFIED** (reuses existing generation during reconnection)
- [x] Keyring operations - **NOT MODIFIED** (no new keyring operations)
- [x] Password transmission to OpenConnect - **NOT MODIFIED** (uses existing connection flow)
- [x] Configuration parsing - **EXTENDED** (adds health check endpoint URL, retry policy parameters - all non-sensitive)

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
akon-core/
├── src/
│   ├── vpn/
│   │   ├── mod.rs
│   │   ├── state.rs                    [MODIFY] Add Reconnecting state
│   │   ├── cli_connector.rs            [MODIFY] Integrate reconnection logic
│   │   ├── connection_event.rs         [MODIFY] Add reconnection events
│   │   ├── network_monitor.rs          [NEW] D-Bus network event detection
│   │   ├── health_check.rs             [NEW] HTTP/HTTPS health check client
│   │   └── reconnection.rs             [NEW] Reconnection state machine & backoff
│   ├── config/
│   │   └── toml_config.rs              [MODIFY] Add reconnection config
│   └── lib.rs
├── tests/
│   ├── network_monitor_tests.rs        [NEW] Mock D-Bus events
│   ├── health_check_tests.rs           [NEW] Mock HTTP responses
│   ├── reconnection_tests.rs           [NEW] State machine & backoff tests
│   └── integration/
│       └── reconnection_flow_tests.rs  [NEW] End-to-end reconnection
src/
├── main.rs                              [MODIFY] Background monitor task
├── cli/
│   └── vpn.rs                           [MODIFY] Status shows reconnection state
└── daemon/
    └── process.rs                       [MODIFY] Process cleanup integration
```

**Structure Decision**: This is a single Rust workspace with a library crate (`akon-core`) and binary crate (`akon`). The reconnection feature will be implemented primarily in `akon-core/src/vpn/` with three new modules:

1. **network_monitor.rs**: Interfaces with NetworkManager via D-Bus to detect network state changes
2. **health_check.rs**: HTTP/HTTPS client for connectivity verification
3. **reconnection.rs**: Core state machine and exponential backoff logic

Existing modules will be extended:

- **state.rs**: Add `Reconnecting { attempt, next_retry_at }` variant to ConnectionState enum
- **cli_connector.rs**: Integrate reconnection triggers and state updates
- **connection_event.rs**: Add events for reconnection lifecycle
- **toml_config.rs**: Parse reconnection policy configuration

## Complexity Tracking

*Fill ONLY if Constitution Check has violations that must be justified*

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
