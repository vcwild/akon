# Research: Test Actors Framework

**Feature**: 005-test-actors-framework
**Date**: 2026-06-21
**Phase**: 0 - Research & Discovery

## Overview

The goal is to make akon's real-world connection behavior testable offline. This requires identifying the exact seams where akon touches the OS/network and choosing a substitution mechanism consistent with the existing codebase.

## Current Untestable Seams (source survey)

| Operation | Location | Why untestable today |
|-----------|----------|----------------------|
| Spawn `sudo openconnect ...` | `akon-core/src/vpn/cli_connector.rs:133` (`spawn_process`) | Needs root + real server |
| Read scripted output stream | `cli_connector.rs:258` (stdout loop) | Driven by a real child's pipes |
| Discover daemon PID via `pgrep` | `cli_connector.rs:88` (`find_openconnect_daemon_pid`) | Needs a real process |
| Signal/terminate via `nix::kill` | `cli_connector.rs:355` (`disconnect`) | Needs a real PID; may need `sudo` |
| Liveness via `ps` | `akon-core/src/vpn/process.rs:32` (`is_process_alive`) | Needs a real process |
| Cleanup via `pgrep` + `kill` | `process.rs:117` (`cleanup_all_openconnect_processes`) | Needs real processes |
| Health check via real HTTP | `akon-core/src/vpn/health_check.rs:125` (`check`) | Needs real network |

## Technical Decisions

### Decision 1: The durable abstraction is a backend-agnostic `VpnBackend` boundary (NOT an openconnect-shaped one)

**Context**: The framework's strategic purpose is to enable **removing the `openconnect` dependency** and replacing it with a native implementation. If the test seam is shaped around openconnect specifics (spawn process / `pgrep` / `kill` / stdout lines), that seam evaporates the moment openconnect is gone, and the scenario suite cannot validate the native backend. The abstraction must be defined in terms akon will *still own* after openconnect: connect, observe lifecycle, disconnect.

**Decision**: Define the primary boundary as a `VpnBackend` trait — roughly `connect(credentials) -> stream of LifecycleEvent`, `disconnect()`, `is_alive()` — using backend-agnostic lifecycle states (`Connecting`, `Authenticating`, `SessionEstablished`, `LinkUp { ip, device }`, `Connected`, `Disconnected`, `Failed`). The current openconnect path becomes `OpenConnectBackend` implementing this trait; a future native backend implements the same trait. The simulated backend (`SimulatedBackend`) also implements it.

**Rationale**: This is the only design that lets the **same scenario suite** validate today's openconnect backend and tomorrow's native backend, which is exactly what makes the migration safe (develop native backend test-first; prove equivalence; then switch the default; then delete openconnect). `SystemEffects` (Decision 1a) is retained but demoted to an *internal* detail of `OpenConnectBackend`.

**Alternatives Considered**:
- *Abstract only OS effects (`SystemEffects`) as the primary seam*: rejected as the primary boundary — it is openconnect-shaped and disappears with openconnect, so it cannot validate a native backend. Kept only as an internal detail of the openconnect backend.
- *No trait, swap implementations via `#[cfg]`*: cannot run two backends in one test for equivalence comparison (US4/FR-014).

### Decision 1a: Keep `SystemEffects` as an internal seam of the openconnect backend

**Context**: `OpenConnectBackend` still needs to spawn/discover/signal a real process today, and those calls must be faked when unit-testing the openconnect backend itself.

**Decision**: Retain a narrow async `SystemEffects` seam (`spawn_vpn`, `discover_pid`, `is_alive`, `signal`) used *inside* `OpenConnectBackend`. It is not part of the public `VpnBackend` contract and is expected to be deleted when openconnect is removed.

**Rationale**: Lets the openconnect backend itself be unit-tested without root, while keeping openconnect specifics out of the durable boundary. Idiomatic substitution seam; matches "explicit interfaces, not shared mutable state".

**Alternatives Considered**:
- *Spawn a fake `openconnect` binary fixture*: brittle, still spawns a real process, slower.
- *Env-var + real binary path swap*: cannot model liveness/signaling without a real process.

### Decision 2: Model the VPN server + openconnect output as a scriptable actor

**Context**: Real connection events come from parsing `openconnect` stdout/stderr line-by-line (`OutputParser`).

**Decision**: `VpnServerActor` holds a script (`Vec<ServerStep>`) of raw output lines / outcomes with optional delays. The simulated spawn returns a stream the connector consumes exactly like real stdout, so `OutputParser` is exercised unchanged.

**Rationale**: Reusing `OutputParser` against scripted lines means the test covers the *real* parsing logic, not a mock of it — maximizing fidelity. The actor pattern mirrors the existing channel-driven `ReconnectionManager`.

**Alternatives Considered**:
- *Emit `ConnectionEvent`s directly*: skips `OutputParser`, lowering fidelity and missing regressions in parsing.

### Decision 3: In-memory process registry for liveness/signaling

**Context**: PID discovery, liveness, and termination need a process to act on.

**Decision**: `FakeProcessRegistry` (an `Arc<Mutex<HashMap<u32, SimProcess>>>`) assigns deterministic PIDs, tracks `Alive`/`Terminated`, and applies SIGTERM/SIGKILL transitions. `SimSystemEffects` wraps it.

**Rationale**: Deterministic, instant, and fully observable. SIGTERM-then-SIGKILL semantics can be scripted (e.g., a process that ignores SIGTERM to test the escalation path).

**Alternatives Considered**:
- *Real short-lived child processes (`sleep`)*: non-deterministic timing, OS-dependent, can't simulate `sudo`-owned PIDs.

### Decision 4: Network actor for reachability, reusing `HealthCheckResult`

**Context**: Reconnection is driven by consecutive health-check failures.

**Decision**: `NetworkActor` exposes a scriptable reachability timeline (`Up`, `Down`, or per-poll sequence). Tests drive reconnection by producing `HealthCheckResult::success/failure` from the actor rather than real HTTP.

**Rationale**: The `ReconnectionManager` already accepts results/commands over channels (`reconnection.rs:226`), so a network actor slots in without changing reconnection logic. Avoids `wiremock`/real sockets for the actor-level scenarios.

**Alternatives Considered**:
- *`wiremock` (already a dev-dep)*: great for HTTP-layer tests and still usable, but it binds a real local socket and tests the HTTP client, not the higher-level reconnection scenario. The network actor is lighter and deterministic for scenario timing. Both can coexist.

### Decision 5: Gate behind a `test-actors` Cargo feature + `cfg(test)`

**Context**: The constitution forbids adding runtime cost/attack surface to released binaries; the repo already uses a `mock-keyring` feature for exactly this.

**Decision**: Put `testkit` behind `#[cfg(any(test, feature = "test-actors"))]`. The `SystemEffects` trait and `RealSystemEffects` are always compiled (production uses them); only the simulated actors are feature-gated.

**Rationale**: Mirrors the proven `mock-keyring` swap (`akon-core/src/auth/mod.rs:8`). Zero cost in release.

### Decision 6: Simulated (logical) time, not wall-clock

**Context**: Real reconnection uses backoff delays; tests must be fast and deterministic.

**Decision**: Scenario delays are expressed logically; the harness advances time via `tokio::time` pause/advance where needed, keeping scenarios in the millisecond range.

**Rationale**: Determinism + speed (SC-001). Avoids flaky sleeps.

## Implementation Patterns

- **Trait-object injection**: `CliConnector::with_effects(Arc<dyn SystemEffects>)`; default constructor uses `RealSystemEffects` (backward compatible).
- **Actor = owned state + channels**: model on `ReconnectionManager` (`reconnection.rs:166`): commands in, observable state/timeline out.
- **Reuse over re-mock**: drive real `OutputParser` and real `ReconnectionManager`; only OS/network leaves are simulated.
- **Timeline recorder**: harness subscribes to events and appends `(logical_time, Observed)` entries; assertions match ordered sub-sequences.

## Best Practices Applied

- Narrow, single-purpose trait (interface segregation).
- Backward-compatible default (existing call sites keep working).
- Feature-gated test code (no release bloat).
- Given/When/Then test structure (matches existing tests).
- No secrets, no network, no root in tests.

## Dependencies

- No new runtime crates. Uses existing `tokio`, `thiserror`. `async-trait` may be added (dev/feature-scoped) if needed for the async trait; alternatively use a hand-written boxed-future or keep the trait methods returning concrete futures. Decision deferred to implementation; prefer avoiding new deps by using `async-trait` only if it is already transitively available, else structure the trait to avoid it.

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Trait extraction subtly changes production connect path | High | Keep `RealSystemEffects` a thin move of existing code; cover with existing connector tests |
| Async trait ergonomics (no `async fn` in traits pre-1.75) | Med | MSRV 1.70 — use `async-trait` or boxed futures; validate it compiles on MSRV |
| Fidelity gap vs. real openconnect output | Med | Script real captured output lines; reuse `OutputParser` |
| Feature flag drift (tests pass only with feature on) | Low | Auto-enable under `cfg(test)`; CI runs default test profile |

## Open Questions

- Should `SystemEffects` also cover health-check HTTP, or keep network simulation at the `HealthCheckResult`/reconnection layer? **Resolved**: keep network simulation at the result/reconnection layer (Decision 4); health-check HTTP stays as-is and is bypassed by feeding results directly in scenarios.

## References

- `akon-core/src/vpn/cli_connector.rs` (spawn/discover/signal)
- `akon-core/src/vpn/process.rs` (liveness/cleanup)
- `akon-core/src/vpn/reconnection.rs:166,226` (actor pattern, command/state channels)
- `akon-core/src/auth/mod.rs:8` (mock-keyring feature-swap precedent)
- `akon-core/src/vpn/output_parser.rs` (reused parsing logic)

## Next Steps

Proceed to Phase 1: data-model.md (entities + state machines), contracts/system-effects-contract.md (trait + actor signatures), quickstart.md (authoring a scenario test).
