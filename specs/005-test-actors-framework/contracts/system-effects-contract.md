# Contracts: Backend Boundary & Test Actors

**Feature**: 005-test-actors-framework
**Phase**: 1 - Design

## Overview

This document specifies the trait/method contracts the implementation must satisfy. The **`VpnBackend` trait is the durable, public contract**; `SystemEffects` is an internal, deletable contract used only by `OpenConnectBackend`.

## VpnBackend (durable boundary)

```rust
#[async_trait::async_trait]
pub trait VpnBackend: Send {
    /// Begin a connection. Returns a receiver of backend-agnostic lifecycle events.
    async fn connect(
        &mut self,
        credentials: Credentials,
    ) -> Result<UnboundedReceiver<LifecycleEvent>, BackendError>;

    /// Tear down the connection (graceful, then forced). Idempotent.
    async fn disconnect(&mut self) -> Result<(), BackendError>;

    /// Whether the connection/tunnel is currently alive.
    fn is_alive(&self) -> bool;

    /// Opaque handle to the live connection (PID today; opaque id for native).
    fn handle(&self) -> Option<ConnectionHandle>;
}
```

**Pre-conditions**: `connect` called once before `disconnect`/`is_alive` are meaningful.
**Post-conditions**:
- A successful connect yields a stream ending in `Connected` (and `is_alive() == true`).
- A failed connect yields a stream ending in `Failed { .. }` and `is_alive() == false`.
- `disconnect` leaves `is_alive() == false` and is a no-op success if already torn down.
**Invariants**: No variant or method name references openconnect specifics. Backend-agnostic only.

## LifecycleEvent (contract vocabulary)

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleEvent {
    Connecting,
    Authenticating,
    SessionEstablished,
    LinkUp { ip: IpAddr, device: String },
    Connected { ip: IpAddr, device: String },
    HealthDegraded,
    Reconnecting { attempt: u32 },
    Disconnected { reason: DisconnectReason },
    Failed { kind: FailureKind, detail: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    Authentication,
    Network,
    ScriptExhausted,
    Backend,
}
```

**Contract**: ordering must respect the state machine in data-model.md. `Connected` MUST be preceded (somewhere upstream) by `Connecting`. `Failed { Authentication }` MUST NOT be preceded by `Connected`.

## SystemEffects (internal to OpenConnectBackend; deletable)

```rust
#[async_trait::async_trait]
pub trait SystemEffects: Send + Sync {
    async fn spawn_vpn(&self, spec: SpawnSpec) -> Result<SpawnedProcess, VpnError>;
    async fn discover_pid(&self, server: &str) -> Option<u32>;
    fn is_alive(&self, pid: u32) -> bool;
    fn signal(&self, pid: u32, sig: TermSignal) -> Result<(), VpnError>;
}
```

- `RealSystemEffects`: `spawn_vpn` = `sudo openconnect ...`; `discover_pid` = `pgrep -f`; `is_alive` = `ps`; `signal` = `nix::kill`. Behavior MUST equal current `cli_connector.rs`/`process.rs`.
- **Not exported** in the public `VpnBackend` contract.

## VpnServerActor (test)

```rust
impl VpnServerActor {
    pub fn new() -> Self;
    pub fn script(steps: Vec<ServerStep>) -> Self;
    /// Convenience scripts:
    pub fn successful_connect(ip: IpAddr, device: &str) -> Self;
    pub fn auth_failure(detail: &str) -> Self;
    pub fn connect_then_drop(ip: IpAddr, device: &str, healthy_polls: u32) -> Self;
    /// Drive the next lifecycle event (used by SimulatedBackend).
    pub async fn next(&mut self) -> Option<LifecycleEvent>;
}
```

**Contract**: emits exactly the scripted sequence; honors `Delay` logically; `successful_connect` ends in `Connected`; `auth_failure` ends in `Failed { Authentication }`.

## FakeTunnelRegistry (test)

```rust
impl FakeTunnelRegistry {
    pub fn new() -> Self;
    pub fn register(&self) -> ConnectionHandle;          // deterministic handles
    pub fn is_alive(&self, h: ConnectionHandle) -> bool;
    pub fn signal(&self, h: ConnectionHandle, sig: TermSignal);
    pub fn set_ignores_graceful(&self, h: ConnectionHandle, v: bool);
}
```

**Contract**: `signal(Forced)` ⇒ `Terminated`; `signal(Graceful)` ⇒ `Terminated` unless `ignores_graceful`, then stays `Terminating` until `Forced`. Never calls real OS.

## NetworkActor (test)

```rust
impl NetworkActor {
    pub fn reachable() -> Self;
    pub fn unreachable() -> Self;
    pub fn script(per_poll: Vec<bool>) -> Self;
    pub fn poll(&mut self) -> HealthCheckResult;   // no real HTTP
}
```

## TestHarness + Timeline (test)

```rust
impl<B: VpnBackend> TestHarness<B> {
    pub fn new(backend: B) -> Self;
    pub async fn run(&mut self, scenario: Scenario) -> Timeline;
}

impl Timeline {
    pub fn events(&self) -> &[LifecycleEvent];
    pub fn assert_reached(&self, e: &LifecycleEvent);
    pub fn assert_subsequence(&self, expected: &[LifecycleEvent]); // ordered, gaps allowed
    pub fn assert_never(&self, e: &LifecycleEvent);
}
```

**Contract**: `assert_subsequence` passes iff `expected` appears as an ordered (not necessarily contiguous) sub-sequence of `events()`; on failure it panics with the expected vs. actual timeline. `run` MUST terminate deterministically (logical timeout → `Failed { ScriptExhausted }`).

## Testing Contracts (the demonstrating tests must prove)

1. **Connect + disconnect**: timeline subsequence `[Connecting, Authenticating, Connected]`; after disconnect `is_alive() == false`; tunnel `Terminated`; no real OS/network touched.
2. **Auth failure**: timeline ends `Failed { Authentication }`; `assert_never(Connected)`; tunnel not alive.
3. **Interruption + reconnect**: subsequence `[Connected, HealthDegraded, Reconnecting, Connected]`.
4. **Cross-backend equivalence**: same scenario run against two `VpnBackend` impls yields equivalent lifecycle subsequence (FR-014).

## Integration Points

- Reuses `OutputParser` inside `OpenConnectBackend` (maps `ConnectionEvent` → `LifecycleEvent`).
- Reuses `ReconnectionManager` semantics for the reconnect scenario (thresholds/backoff) where practical; otherwise the harness drives reconnection via `NetworkActor` + backend `connect` retries.

## Backward Compatibility

- Existing `CliConnector` public API remains; `OpenConnectBackend` wraps it. Production call sites may keep using `CliConnector` directly during the transition, or move to `OpenConnectBackend`. No CLI/behavior change in release builds.

## Summary

`VpnBackend` + `LifecycleEvent` form the public, durable contract enabling backend swap and openconnect removal. `SystemEffects` is an internal, deletable seam. Actors + harness + timeline provide deterministic, offline, backend-independent scenario execution.
