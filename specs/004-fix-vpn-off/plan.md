# Implementation Plan: Fix VPN Off Command Cleanup

**Branch**: `004-fix-vpn-off` | **Date**: 2025-11-08 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/004-fix-vpn-off/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

The `akon vpn off` command currently only terminates the tracked OpenConnect process but doesn't clean up orphaned processes from previous sessions, leading to residual connections. This plan merges the existing `cleanup_orphaned_processes()` functionality into the `vpn off` command, ensuring comprehensive cleanup and simplifying the user workflow by eliminating the need for a separate `akon vpn cleanup` command.

## Technical Context

**Language/Version**: Rust 1.70+ (stable channel, edition 2021)

**Primary Dependencies**:

- `nix` (for signal handling: SIGTERM, SIGKILL)
- `tokio` (async runtime for timeout handling)
- `serde_json` (state file serialization)
- `tracing` (structured logging)
- `colored` (CLI output formatting)

**Storage**: JSON state file at `/tmp/akon_vpn_state.json` (tracks VPN connection state including PID)

**Testing**: Rust standard testing framework (`cargo test`), integration tests in `tests/` directory

**Target Platform**: Linux (requires `pgrep`, `ps`, `kill` utilities and nix signal APIs)

**Project Type**: Single project (CLI application with library crate)

**Performance Goals**:

- Graceful shutdown attempt within 5 seconds
- Total command completion within 15 seconds (including force-kill)
- Zero residual processes after completion

**Constraints**:

- Must handle root-owned OpenConnect processes (requires sudo for kill operations)
- Must not hang indefinitely on unresponsive processes
- Must maintain backward compatibility with existing state file format

**Scale/Scope**:

- Single command modification (`run_vpn_off`)
- Reuse existing `cleanup_orphaned_processes()` function from `src/daemon/process.rs`
- Deprecate/remove `run_vpn_cleanup` command
- Update CLI help text and documentation

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.0.0:

- [x] **Security-First**: No credentials involved in this feature - only process termination. State file contains no secrets (only PID, IP, timestamp). No security impact.
- [x] **Modular Architecture**: Changes isolated to `src/cli/vpn.rs` and reuses existing `cleanup_orphaned_processes()` from `src/daemon/process.rs`. Clear separation maintained.
- [x] **Test-Driven Development**: Existing tests in `tests/vpn_disconnect_tests.rs` will be extended. New test cases will verify comprehensive cleanup behavior.
- [x] **Observability**: Cleanup operations already logged via `tracing` crate. State transitions (disconnecting, cleanup complete) will be logged at INFO level.
- [x] **CLI-First Interface**: Maintaining CLI interface `akon vpn off`. Removing redundant `akon vpn cleanup` command simplifies interface.

**Security-Critical Changes** (require extra scrutiny):

- [ ] OAuth token handling - N/A
- [ ] OTP generation algorithm - N/A
- [ ] Keyring operations - N/A
- [ ] Password transmission to OpenConnect - N/A
- [ ] Configuration parsing (public vs. secret separation) - N/A

**Notes**: This is a bug fix that improves reliability of the disconnect operation without touching any security-critical components. No constitutional violations.

## Project Structure

### Documentation (this feature)

```text
specs/004-fix-vpn-off/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
src/
├── main.rs              # Entry point - no changes needed
├── cli/
│   ├── mod.rs
│   ├── setup.rs
│   ├── get_password.rs
│   └── vpn.rs           # MODIFY: run_vpn_off(), REMOVE/DEPRECATE: run_vpn_cleanup()
└── daemon/
    ├── mod.rs
    ├── ipc.rs
    └── process.rs       # REFERENCE: cleanup_orphaned_processes() - no changes needed

akon-core/
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── types.rs
│   ├── auth/
│   ├── config/
│   └── vpn/
│       ├── cli_connector.rs
│       ├── health_check.rs
│       ├── process.rs
│       └── reconnection.rs

tests/
├── vpn_disconnect_tests.rs     # MODIFY: Add comprehensive cleanup tests
├── integration/
│   └── vpn_disconnect_tests.rs # MODIFY: Add integration tests for cleanup
└── unit/
```

**Structure Decision**: Single project structure with CLI application and library crate. Changes isolated to `src/cli/vpn.rs` with new tests in `tests/` directory. The existing `cleanup_orphaned_processes()` function in `src/daemon/process.rs` will be called from the modified `run_vpn_off()` function.

## Complexity Tracking

No constitutional violations. This is a straightforward bug fix that:

- Reuses existing, tested code (`cleanup_orphaned_processes()`)
- Simplifies the CLI by removing redundant command
- Maintains existing architectural boundaries
- Requires no new dependencies or infrastructure
