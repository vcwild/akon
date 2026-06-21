<!--
SYNC IMPACT REPORT
==================
Version: 1.0.0 → 1.1.0 (Add Principle VI: Test Actors & Seam-Isolated Testing)
Date: 2026-06-21

CHANGES:
- Added Principle VI: Test Actors & Seam-Isolated Testing (NON-NEGOTIABLE),
  codifying the methodology proven while building the test actors framework
  (spec 005) and the native F5 backend (spec 006): isolate heavy/real-world
  integrations (OS, network, TLS, processes) behind seams; emulate them with
  in-memory actors as ground truth; validate behavior offline and
  deterministically; then confirm with one bounded REAL end-to-end test on the
  production path before acknowledging a replacement.
- Expanded Development Standards with a "Test Methodology" subsection
  (seams, actors, backend-agnostic boundaries, no-hang discipline,
  real end-to-end confirmation).
- Updated Governance code-review checklist to require seam/actor compliance and
  hang-proof tests.

PRINCIPLES DEFINED:
1. Security-First Architecture
2. Modular Architecture
3. Test-Driven Development (NON-NEGOTIABLE)
4. Observability & Logging
5. CLI-First Interface
6. Test Actors & Seam-Isolated Testing (NON-NEGOTIABLE)  ← NEW

TEMPLATES REQUIRING UPDATES:
✅ plan-template.md - Constitution Check updated to include Principle VI and bump version reference
✅ spec-template.md - Testing/seam requirements align (no structural change required)
✅ tasks-template.md - Task categorization supports seam/actor tests (no structural change required)

FOLLOW-UP TODOS:
- Pre-existing drift: constitution still references Python tooling (mypy/ruff/pytest,
  *.py modules) although the codebase is Rust. Not addressed in this amendment;
  track separately.
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

### VI. Test Actors & Seam-Isolated Testing (NON-NEGOTIABLE)

**Every behavior that depends on a heavy or real-world integration MUST be testable offline, deterministically, and without hanging — by isolating the integration behind a seam and emulating it with an in-memory test actor that serves as ground truth.**

This principle codifies the methodology that produced the test actors framework (`akon-core/src/vpn/testkit/`) and the native F5 backend. It applies to anything that would otherwise require real infrastructure to test: the operating system (process spawn/signal/discovery, TUN devices, routing), the network (TLS sockets, HTTP endpoints, DTLS, reachability), external binaries (`openconnect`, `pgrep`, `kill`), root privileges, or wall-clock time.

- **Seams over real I/O**: Heavy integrations MUST be accessed through an explicit interface (a Rust trait such as `Transport`, `TunDevice`, `SystemEffects`, or `VpnBackend`) — never via hard-coded direct calls scattered through logic. Each seam has a real production implementation and a test implementation.
- **Durable, behavior-shaped boundaries**: The primary abstraction MUST be expressed in terms the project will still own after a dependency is removed (e.g. connection lifecycle events), NOT in terms of the current implementation's artifacts (e.g. a child process's stdout). Implementation-specific seams (like `SystemEffects` for the openconnect path) are permitted but MUST be internal details of one implementation and deletable with it.
- **Actors as ground truth**: Test implementations of seams MUST be in-memory actors (a fake server, a peer, a registry, a controllable network) that emulate real behavior faithfully and reuse the real codecs/state machines wherever possible (e.g. the fake F5 server drives the genuine framing/PPP code). They MUST perform no real I/O, require no root, and never touch the host network.
- **Backend-agnostic scenario suites**: When replacing a component, the SAME scenario suite MUST validate the old and new implementations against the shared boundary, and equivalence MUST be demonstrated before the replacement may become the default.
- **No-hang discipline**: Tests MUST be deterministic and bounded. Every wait on I/O MUST have a timeout; every in-memory transport/channel MUST signal EOF/close (including on drop) so consumer loops terminate. A test that can hang is a defect, not an inconvenience — the fix is to bring the integration into the actors model, not to leave a blocking test.
- **Real end-to-end confirmation**: Emulation proves protocol/logic correctness; it does not by itself acknowledge a replacement. A replacement of a real integration MUST also be confirmed by at least one **real** end-to-end test that exercises the production seam implementation (e.g. a genuine TLS-over-TCP handshake against a local server), kept bounded so it cannot hang and self-contained so it needs no external infrastructure, root, or non-loopback network.
- **Feedback loop**: When something is too complex or too heavy to test, the required response is to extend the actors model with the missing capability (a new seam or actor), then test against it — iterating until the behavior is covered. Writing a slow, flaky, or hanging test instead is prohibited.
- **Zero release cost**: Test actors and in-memory implementations MUST be gated out of release builds (e.g. behind a `test-actors` feature / `cfg(test)`), so they add no runtime cost or attack surface to shipped binaries. The seam traits and real implementations remain in production.

**Rationale**: akon's core job — establishing VPN tunnels — is exactly the kind of behavior that is expensive, privileged, and disruptive to test against reality (it needs a server, root, and would drop the developer's own connectivity). Seam isolation plus in-memory actors make that behavior testable on every change, while a single bounded real end-to-end test guards against the divergence between emulation and production I/O (such as TLS read coalescing). This is what makes risky changes — above all, removing the `openconnect` dependency — safe to develop test-first and prove equivalent before shipping.

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

### Test Methodology (Principle VI in practice)

This section is the operational guide for satisfying Principle VI. It is the default way features touching the OS, network, processes, TLS, or privileged operations are built.

- **Identify the seam first.** Before implementing anything that does real I/O, define the trait that abstracts it (read/write byte stream, OS effects, TUN device, connection backend). Logic depends on the trait, not on concrete sockets/commands.
- **Pure layers stay pure.** Decompose protocols into pure, deterministic units (framing/codecs, state machines, parsers) that are testable with byte-exact vectors and need no I/O at all. Validate these against ground truth (e.g. the reference implementation's wire format) with explicit test vectors.
- **Provide two implementations per seam.** A real one (production) and an in-memory actor (test). The actor reuses the real pure layers so tests exercise genuine code, not a re-mock of it.
- **Drive tests with scenarios, not ad-hoc setup.** Compose real-world situations declaratively and assert on an ordered timeline of observable, backend-agnostic events. Reuse one scenario suite across implementations to prove equivalence.
- **Bound everything.** Wrap handshakes/loops in `tokio::time::timeout`; ensure in-memory transports/channels report EOF on close and on drop. No unbounded `recv`/`read` without a deadline.
- **Confirm on the real path.** Add at least one bounded, self-contained real end-to-end test (e.g. a local TLS server on loopback with a self-signed cert) for any replacement of a real integration. This is what catches emulation/production divergence (e.g. TLS coalescing post-`/myvpn` PPP bytes).
- **Iterate the framework, not the workaround.** If a behavior can't be tested cleanly, extend the actors framework with the missing seam/actor and circle back — never settle for a slow, flaky, or hanging test.
- **Gate test code out of releases.** Keep actors/in-memory impls behind a test feature/`cfg(test)`; ship only seams + real implementations.

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
- **Seam & test-actor compliance (Principle VI)**: heavy/real integrations are behind a seam with an in-memory actor; behavior is tested offline and deterministically; replacements include a bounded real end-to-end test; no test can hang; test-only code is gated out of release builds.

Complexity that violates modularity principles MUST be justified in commit messages or rejected.

**Version**: 1.1.0 | **Ratified**: 2025-10-08 | **Last Amended**: 2026-06-21
