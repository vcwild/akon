# Implementation Plan: Test Actors Framework

**Branch**: `005-test-actors-framework` | **Date**: 2026-06-21 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/005-test-actors-framework/spec.md`

## Summary

Introduce a **backend-agnostic `VpnBackend` boundary** — "connect, observe lifecycle, disconnect" — as the durable abstraction that will outlive `openconnect`. The current openconnect-delegation logic becomes one backend (`OpenConnectBackend`) behind that trait, with its OS-touching operations (spawn via `sudo`, `pgrep`/`ps`, `kill`, stdout parsing) confined to it via an internal `SystemEffects` seam. Add a **simulated backend** backed by in-memory actors — a scriptable VPN server actor, a fake tunnel/process registry, and a controllable network actor — plus a `TestHarness` + scenario builder that records an assertable, backend-independent timeline.

This serves two goals at once: (1) real-world scenarios (connect, auth failure, silent tunnel death, reconnection) become testable deterministically and offline, with no root, no real `openconnect`, and no impact on the developer's internet; and (2) the **same scenario suite can be run against any backend**, so a future native VPN backend (no external dependencies) can be developed test-first and proven behaviorally equivalent *before* `openconnect` is removed.

The framework lives behind a `test-actors` Cargo feature (auto-enabled under `cfg(test)`), mirroring the existing `mock-keyring` feature-swap pattern, so released binaries are unaffected. The `VpnBackend` trait and `OpenConnectBackend` are always compiled (production uses them); only the simulated backend + actors are feature-gated.

## Technical Context

**Language/Version**: Rust 2021, MSRV 1.70
**Primary Dependencies**: tokio (sync/time/process), thiserror; reuses existing `ConnectionEvent`, `ConnectionState`, `OutputParser`, `ReconnectionManager`. No new runtime dependencies.
**Storage**: N/A (in-memory only)
**Testing**: `cargo test` (akon-core integration tests + inline unit tests); follows existing Given/When/Then convention
**Target Platform**: Linux (dev + CI), offline-capable
**Project Type**: single (Cargo workspace: `akon-core` library + `akon` binary)
**Performance Goals**: Test scenarios complete in milliseconds; uses simulated/compressed time, not wall-clock waits
**Constraints**: Must never touch real OS/network in harness-driven tests; zero runtime cost in release builds; additive only (no behavior change to production path)
**Scale/Scope**: New `akon-core/src/testkit/` module (~5 files) + 1 integration test file; a `SystemEffects` trait extraction with one real and one simulated impl.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.1.0:
(Note: this feature is the origin of Principle VI — Test Actors & Seam-Isolated Testing — which was codified into the constitution based on the methodology established here.)

- [x] **Security-First**: No credentials handled by the framework; simulated actors carry no real secrets. The framework is gated out of release builds, adding no attack surface. No plaintext secrets in code/config/logs.
- [x] **Modular Architecture**: Introduces a clean `SystemEffects` boundary (explicit interface, not shared mutable state) decoupling orchestration from OS effects. Each actor (server, process registry, network) is an independent, single-responsibility module.
- [x] **Test-Driven Development**: This feature *is* test infrastructure. It directly advances TDD by making connect/reconnect logic testable; the framework itself ships with tests proving its actors behave correctly.
- [x] **Observability**: The harness records an ordered timeline; simulated state changes are observable and assertable. No secrets logged.
- [x] **CLI-First Interface**: No CLI surface change. Production CLI behavior is unchanged; framework is internal test tooling only.
- [x] **Test Actors & Seam-Isolated Testing**: This feature *establishes* the methodology — the `VpnBackend` durable boundary, the in-memory actors (server, network, tunnel registry), the backend-agnostic scenario suite, and the no-hang discipline (EOF-on-drop). All test actors are gated behind `test-actors`/`cfg(test)`.

**Security-Critical Changes** (require extra scrutiny):
- [ ] OAuth token handling — N/A
- [ ] OTP generation algorithm — N/A
- [ ] Keyring operations — N/A
- [ ] Password transmission to OpenConnect — N/A (simulated server does not validate real passwords; no real transmission)
- [ ] Configuration parsing (public vs. secret separation) — N/A

**Notes**: Purely additive test infrastructure. The one production-affecting change is extracting a `SystemEffects` trait and routing the existing `CliConnector`/process code through it; the real implementation preserves current behavior byte-for-byte in the connect/disconnect path.

## Project Structure

### Documentation (this feature)

```
specs/005-test-actors-framework/
├── spec.md              # Feature spec
├── plan.md              # This file
├── research.md          # Phase 0: design decisions
├── data-model.md        # Phase 1: entities, state machines
├── quickstart.md        # Phase 1: how to write a scenario test
├── contracts/
│   └── system-effects-contract.md   # Trait + actor contracts
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 task list
```

### Source Code (repository root)

```
akon-core/
├── Cargo.toml                       # ADD: `test-actors` feature
└── src/
    └── vpn/
        ├── mod.rs                   # MODIFY: expose backend + testkit (feature-gated)
        ├── backend.rs               # NEW: VpnBackend trait + LifecycleEvent (backend-agnostic boundary)
        ├── system_effects.rs        # NEW: SystemEffects seam (INTERNAL to openconnect backend)
        ├── openconnect_backend.rs   # NEW: OpenConnectBackend impl wrapping today's CliConnector path
        ├── cli_connector.rs         # MODIFY: route spawn/discover/signal through SystemEffects
        ├── process.rs               # REFERENCE: real impl source of truth
        └── testkit/                 # NEW: the test actors framework (feature-gated)
            ├── mod.rs               # Re-exports
            ├── server_actor.rs      # VpnServerActor (scriptable backend-agnostic lifecycle)
            ├── sim_backend.rs       # SimulatedBackend (impl VpnBackend) + fake tunnel/process registry
            ├── network_actor.rs     # NetworkActor (reachability over time)
            ├── scenario.rs          # Scenario + ScenarioBuilder (backend-independent)
            └── harness.rs           # TestHarness<B: VpnBackend> + Timeline + assertions

akon-core/tests/
└── test_actors_framework_tests.rs   # NEW: the demonstrating tests (incl. cross-backend equivalence)
```

**Structure Decision**: Single Cargo workspace, library-centric. The framework lives inside `akon-core` (not a separate crate) so it can construct and drive the real internal types (`ConnectionEvent`, `ReconnectionManager`) directly. The **`VpnBackend` trait is the primary, durable abstraction** — it is what a future native backend will implement and what removing `openconnect` depends on. `SystemEffects` is demoted to an internal detail of `OpenConnectBackend` (it disappears with openconnect). The harness is generic over `B: VpnBackend` so one scenario runs against any backend. Simulated backend + actors are gated behind the `test-actors` feature and `cfg(test)`, following the established `mock-keyring` pattern, keeping them out of release binaries.

## Complexity Tracking

*No constitution violations. Table intentionally empty.*

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
