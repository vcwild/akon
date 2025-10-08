<!--
SYNC IMPACT REPORT
==================
Version: 0.0.0 → 1.0.0 (Initial Constitution)
Date: 2025-10-08

CHANGES:
- Initial constitution creation for auto-openconnect (akon) project
- Established 5 core principles: Security-First, Modular Architecture, Test-Driven Development,
  Observability & Logging, CLI-First Interface
- Added Security Requirements section
- Added Development Standards section
- Defined governance and amendment procedures

PRINCIPLES DEFINED:
1. Security-First Architecture
2. Modular Architecture
3. Test-Driven Development (NON-NEGOTIABLE)
4. Observability & Logging
5. CLI-First Interface

TEMPLATES REQUIRING UPDATES:
✅ plan-template.md - Constitution Check section aligns with all 5 principles
✅ spec-template.md - Security and testing requirements align
✅ tasks-template.md - Task categorization supports security, testing, and modularity
✅ commands/*.md - Generic guidance preserved, no agent-specific references

FOLLOW-UP TODOS: None
-->

# Auto-OpenConnect (Akon) Constitution

## Core Principles

### I. Security-First Architecture

**All credential storage and handling MUST prioritize security above convenience.**

- Sensitive data (OAuth tokens, PINs, OTP seeds) MUST be stored exclusively in GNOME Keyring or equivalent secure storage—never in plaintext files, environment variables, or logs.
- OTP token generation MUST use cryptographically secure algorithms (TOTP with HMAC-SHA1/SHA256).
- Password transmission to OpenConnect MUST use secure channels (stdin with `--passwd-on-stdin`).
- Configuration files MUST separate public settings (VPN server, username, protocol) from secrets.
- All credential operations MUST be auditable through structured logging (excluding sensitive values).

**Rationale**: As a VPN connector handling enterprise authentication, any credential compromise could expose corporate networks. Security failures are system-critical bugs.

### II. Modular Architecture

**Core functionality MUST be decomposed into independent, composable modules.**

- Authentication (`auth.py`): Keyring operations, credential retrieval, OTP generation—independently testable without VPN connection.
- Configuration (`config.py`): TOML parsing, settings validation—testable with mock files.
- Connection (`connect.py`, `exec.py`): OpenConnect process management—mockable for testing.
- Monitoring (`monitor.py`): Network event detection, reconnection logic—testable with simulated events.
- Each module MUST have a single, well-defined responsibility with clear boundaries.
- Modules MUST communicate through explicit interfaces, not shared mutable state.

**Rationale**: Modularity enables isolated testing of security-critical components (OTP generation, keyring access) without requiring live VPN infrastructure.

### III. Test-Driven Development (NON-NEGOTIABLE)

**All code changes MUST follow red-green-refactor TDD cycle.**

- Write failing tests demonstrating new behavior or bug reproduction.
- Implement minimal code to pass tests.
- Refactor while keeping tests green.
- **Security-critical modules** (auth, OTP generation, keyring operations) MUST achieve >90% code coverage.
- **Integration tests** MUST verify end-to-end flows: keyring → OTP generation → OpenConnect execution.
- **Test categories required**:
  - Unit tests: Pure logic (OTP algorithm, config parsing)
  - Integration tests: External dependencies (keyring, file I/O)
  - System tests: OpenConnect subprocess mocking

**Rationale**: TDD prevents regression in security and connection logic, where manual testing is expensive and credential-dependent.

### IV. Observability & Logging

**All operations MUST be traceable through structured, security-aware logging.**

- Use systemd journal integration (`journalctl -t AUTO-VPN`) for centralized log collection.
- Log levels MUST follow: DEBUG (detailed flow), INFO (state changes), WARNING (recoverable errors), ERROR (failures).
- **Never log** OAuth tokens, PINs, generated OTP values, or passwords.
- Log security-relevant events: keyring access attempts, OTP generation requests, connection state transitions, authentication failures.
- VPN monitor MUST log reconnection decisions with context: network change, suspend/resume, idle timeout.
- Errors MUST include actionable context: missing config keys, keyring backend failures, OpenConnect exit codes.

**Rationale**: Automated VPN reconnection requires observable state to diagnose failures without interactive debugging.

### V. CLI-First Interface

**All functionality MUST be accessible via command-line interface with composable outputs.**

- Primary commands: `akon` (connect), `akon vpn {on|off|status}`, `akon get-password`, `akon setup-keyring`.
- Support both human-readable output (emoji status, formatted messages) and machine-parsable output (exit codes, structured logs).
- Scripts (Bash wrapper) MUST delegate to Python CLI, not reimplement logic.
- CLI MUST support `--config` flag to override default config location (`~/.config/akon/config.toml`).
- Password generation (`get-password`) MUST output only the password to stdout for piping, errors to stderr.

**Rationale**: CLI-first design enables automation (systemd timers, NetworkManager dispatchers) and scripting without GUI dependencies.

## Security Requirements

### Credential Isolation

- **Secrets MUST NOT** be committed to version control (`.gitignore` enforcement).
- **Config files** MUST use TOML format with clear separation of public settings and secret references.
- **Environment variables** MUST NOT store secrets directly—only config file paths.

### Keyring Backend Validation

- On unsupported platforms (no GNOME Keyring), setup MUST fail with clear error and setup instructions.
- Keyring operations MUST handle backend failures gracefully (prompt for manual intervention, not crash).

### Audit Trail

- All keyring access (set/get/delete) MUST be logged with operation type and key name (not value).
- Failed authentication attempts MUST be logged with sanitized error details.

## Development Standards

### Code Quality

- **Type annotations** MUST be complete for all public APIs (enforced by `mypy --strict`).
- **Python 3.13+** required for latest typing features and performance.
- **Linting** with `ruff` MUST pass on all commits (format + check).
- **Dependencies** MUST be minimal: `cysystemd` (logging), `keyring` (secrets), `secretstorage` (GNOME backend).

### Testing Gates

- All PRs MUST pass: unit tests (pytest), type checking (mypy), linting (ruff), integration tests (keyring/file I/O).
- Security-critical modules MUST have dedicated test files: `test_auth.py`, `test_keyring_utils.py`, `test_password_generator.py`.

### Documentation

- README MUST include: quick start, security best practices, troubleshooting, configuration examples.
- Inline docstrings MUST explain security-relevant design decisions (why keyring, not files).
- Configuration schema MUST be documented in README with example TOML.

### Visual Documentation

- **All diagrams and flowcharts MUST use Mermaid syntax** for maintainability and version control.
- Architecture diagrams, state machines, sequence diagrams, and process flows MUST be embedded in Markdown as Mermaid code blocks.
- ASCII art diagrams are NOT permitted and existing ones MUST be converted to Mermaid.
- Mermaid diagrams MUST be rendered inline in documentation tools (GitHub, GitLab, VS Code preview).
- **Rationale**: Mermaid enables text-based diagrams that are diff-friendly, version-controlled, and automatically rendered without external tools.

## Governance

This constitution supersedes all other development practices and guides. Amendments require:

1. **Proposal**: Document rationale, affected principles, migration plan.
2. **Review**: Evaluate impact on security posture, test coverage, and user trust.
3. **Approval**: Maintainer sign-off with updated version number.
4. **Migration**: Update templates, tests, and documentation to reflect changes.

All code reviews MUST verify:

- Security principle compliance (no plaintext secrets, keyring usage).
- Test coverage for new code paths.
- Logging completeness for state changes.
- CLI interface consistency (exit codes, output format).

Complexity that violates modularity principles MUST be justified in commit messages or rejected.

**Version**: 1.0.0 | **Ratified**: 2025-10-08 | **Last Amended**: 2025-10-08
