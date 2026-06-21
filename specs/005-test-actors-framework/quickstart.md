# Quickstart: Test Actors Framework

**Feature**: 005-test-actors-framework
**Date**: 2026-06-21
**For**: Developers writing real-world scenario tests against the VPN backend

## 🎯 What You're Building

A real-world scenario test that drives akon's connection logic against a **simulated VPN backend** instead of a real `openconnect` process. You describe a scenario declaratively with a `ScenarioBuilder`, run it through a `TestHarness<SimulatedBackend>`, and assert on a recorded `Timeline` of **backend-agnostic** `LifecycleEvent` values — all under a plain `cargo test`, with **no root, no real `openconnect`, and no real network**.

The same scenarios you write today will later validate a **native (no-openconnect) backend** through the shared `VpnBackend` trait — that's the strategic payoff, not just convenience.

## 📋 Quick Context

**Problem**: akon's most important behaviors — connect, auth failure, silent tunnel death, reconnection — all touch the live OS and live network. They can't be exercised in automated tests without root, a real VPN endpoint, and a connection that would knock the developer offline.

**Solution**: A `VpnBackend` boundary ("connect, observe lifecycle, disconnect") with a `SimulatedBackend` backed by in-memory actors (server, fake tunnel registry, network). A `TestHarness` runs declarative scenarios and records an assertable `Timeline`.

**Impact**: Real-world regression tests run deterministically and offline. Because scenarios are expressed in backend-agnostic terms, the same suite proves a future native backend behaves identically *before* `openconnect` is removed.

## 🛠️ Implementation Steps

### Step 1: Pick the Scenario You Want to Test

Decide which real-world situation you're regression-testing. The framework ships convenience scripts for the common ones:

- Successful connect + clean disconnect
- Authentication failure
- Network interruption followed by successful reconnection

Everything is expressed as **backend-independent test data** — you never reference `pgrep`, `kill`, `sudo`, or stdout lines.

### Step 2: Compose the Scenario with `ScenarioBuilder`

**File**: `akon-core/tests/test_actors_framework_tests.rs`

Use the fluent builder to describe the situation as a sequence of steps. The builder produces a `Scenario` that any backend can run:

```rust
use akon_core::vpn::backend::LifecycleEvent;
use akon_core::vpn::backend::FailureKind;
use akon_core::vpn::testkit::{ScenarioBuilder, TestHarness, SimulatedBackend, VpnServerActor, NetworkActor};

use std::net::{IpAddr, Ipv4Addr};

#[tokio::test]
async fn interruption_then_reconnect_returns_to_connected() {
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

    // 1. Describe the scenario declaratively (backend-independent test data).
    let scenario = ScenarioBuilder::new()
        .connect()
        .stay_healthy(3)        // 3 healthy polls
        .drop_network(2)        // link drops for 2 polls
        .expect_reconnect()     // failure threshold → reconnect
        .stay_healthy(1)        // recovered
        .disconnect()
        .build();

    // 2. Wire a simulated backend (server actor + fake tunnel registry).
    let server = VpnServerActor::connect_then_drop(ip, "tun0", 3);
    let backend = SimulatedBackend::new(server, NetworkActor::script(vec![
        true, true, true,   // healthy
        false, false,       // dropped
        true,               // recovered
    ]));

    // 3. Run the scenario and record an ordered timeline of observed events.
    let mut harness = TestHarness::new(backend);
    let timeline = harness.run(scenario).await;

    // 4. Assert on the backend-agnostic lifecycle — ordered sub-sequence, gaps allowed.
    timeline.assert_subsequence(&[
        LifecycleEvent::Connecting,
        LifecycleEvent::Authenticating,
        LifecycleEvent::Connected { ip, device: "tun0".to_string() },
        LifecycleEvent::HealthDegraded,
        LifecycleEvent::Reconnecting { attempt: 1 },
        LifecycleEvent::Connected { ip, device: "tun0".to_string() },
    ]);

    // 5. The simulated tunnel must be torn down after disconnect — no real `kill`.
    assert!(!harness.backend().is_alive());
}
```

### Step 3: Assert Auth Failure with the Same Vocabulary

An authentication failure ends in `Failed { Authentication }` and **never** reaches `Connected`:

```rust
#[tokio::test]
async fn authentication_failure_never_connects() {
    let scenario = ScenarioBuilder::new()
        .connect()
        .expect_auth_failure()
        .build();

    let server = VpnServerActor::auth_failure("invalid OTP");
    let backend = SimulatedBackend::new(server, NetworkActor::reachable());

    let mut harness = TestHarness::new(backend);
    let timeline = harness.run(scenario).await;

    // The flow ends in a backend-agnostic auth failure...
    timeline.assert_reached(&LifecycleEvent::Failed {
        kind: FailureKind::Authentication,
        detail: "invalid OTP".to_string(),
    });
    // ...and never reaches Connected; no tunnel left alive.
    timeline.assert_never(&LifecycleEvent::Connected {
        ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        device: "tun0".to_string(),
    });
    assert!(!harness.backend().is_alive());
}
```

### Step 4: Run It

No special setup. No `sudo`. No VPN server. No network changes:

```bash
cargo test -p akon-core test_actors_framework
```

The `test-actors` Cargo feature is **auto-enabled under `cfg(test)`** (mirroring the existing `mock-keyring` feature-swap pattern), so the simulated backend and actors are available to tests without you enabling anything — and they're compiled **out** of release builds entirely.

## ✅ Definition of Done

Before considering a new scenario test complete, verify:

- [ ] Scenario is composed via `ScenarioBuilder` — no edits to production source modules
- [ ] Assertions use only backend-agnostic `LifecycleEvent` values (no `pgrep`/`kill`/stdout/`sudo` references)
- [ ] Happy-path tests assert the `[Connecting, Authenticating, Connected]` sub-sequence
- [ ] Failure tests assert `Failed { Authentication }` and `assert_never(Connected)`
- [ ] Reconnect tests assert `[Connected, HealthDegraded, Reconnecting, Connected]`
- [ ] `is_alive()` is `false` after `disconnect()`; the simulated tunnel is `Terminated`
- [ ] The test runs green under a plain `cargo test` with no root and no network access
- [ ] No real `sudo`, `openconnect`, `pgrep`, `ps`, `kill`, or outbound HTTP is invoked
- [ ] (When applicable) the same scenario also runs against a second `VpnBackend` impl and yields an equivalent timeline

## 🧪 Manual Testing Script

```bash
#!/bin/bash
# Prove the framework is offline, root-free, and network-safe.

# 1. Record current connectivity (the suite must NOT change it).
ip route show > /tmp/akon_routes_before.txt

# 2. Run the framework tests with NO sudo, NO VPN, NO special network setup.
cargo test -p akon-core test_actors_framework

# 3. Confirm no real openconnect process was spawned by the suite.
if [ "$(pgrep -x openconnect | wc -l)" -eq 0 ]; then
    echo "✅ SUCCESS: No real openconnect process spawned"
else
    echo "❌ FAILURE: A real openconnect process exists"
    pgrep -x openconnect
fi

# 4. Confirm routing/connectivity is unchanged (developer kept internet access).
ip route show > /tmp/akon_routes_after.txt
if diff -q /tmp/akon_routes_before.txt /tmp/akon_routes_after.txt > /dev/null; then
    echo "✅ SUCCESS: Host routing unchanged"
else
    echo "❌ FAILURE: Host routing was modified"
    diff /tmp/akon_routes_before.txt /tmp/akon_routes_after.txt
fi
```

## 📚 Key Files Reference

| File | Purpose | Changes |
|------|---------|---------|
| `akon-core/src/vpn/backend.rs` | `VpnBackend` trait + `LifecycleEvent` (backend-agnostic, durable boundary) | New — always compiled |
| `akon-core/src/vpn/system_effects.rs` | `SystemEffects` seam (internal to the openconnect backend; deletable) | New |
| `akon-core/src/vpn/openconnect_backend.rs` | `OpenConnectBackend` wrapping today's `CliConnector` path | New |
| `akon-core/src/vpn/testkit/server_actor.rs` | `VpnServerActor` — scriptable backend-agnostic lifecycle | New — feature-gated |
| `akon-core/src/vpn/testkit/sim_backend.rs` | `SimulatedBackend` (impl `VpnBackend`) + fake tunnel/process registry | New — feature-gated |
| `akon-core/src/vpn/testkit/network_actor.rs` | `NetworkActor` — reachability over time | New — feature-gated |
| `akon-core/src/vpn/testkit/scenario.rs` | `Scenario` + `ScenarioBuilder` (backend-independent) | New — feature-gated |
| `akon-core/src/vpn/testkit/harness.rs` | `TestHarness<B: VpnBackend>` + `Timeline` + assertions | New — feature-gated |
| `akon-core/tests/test_actors_framework_tests.rs` | The demonstrating tests (incl. cross-backend equivalence) | New |

## 🚀 Estimated Effort

- **Author a new scenario test**: 15-30 minutes (compose builder + assert timeline)
- **Add a new convenience server script**: 30-45 minutes (script `Vec<ServerStep>` + helper)
- **Run a scenario against a second backend (cross-backend equivalence)**: 30 minutes (no scenario/harness changes — implement the trait, reuse the suite)
- **Total for a typical regression test**: under an hour

## 💡 Tips & Gotchas

1. **Backend-agnostic only**: Never assert on openconnect artifacts (PIDs from `pgrep`, stdout strings, `sudo`). If your assertion would break after `openconnect` is removed, it's wrong. Assert on `LifecycleEvent` instead.

2. **`assert_subsequence` allows gaps**: It matches an *ordered, not necessarily contiguous* sub-sequence. Assert the events that matter; intermediate events won't fail the match.

3. **Logical time, not wall-clock**: Scenarios use compressed/logical time for delays. Don't add real `sleep`s — `stay_healthy(n)` / `drop_network(n)` count logical polls, not seconds.

4. **Deterministic termination**: If a script is exhausted before a terminal event, the harness surfaces `Failed { ScriptExhausted }` rather than hanging. A hanging test is a script bug.

5. **Disconnect is idempotent**: Disconnecting an already-terminated tunnel is a no-op success, mirroring production. `is_alive()` must be `false` afterward.

6. **Feature seam is automatic**: `test-actors` is auto-enabled under `cfg(test)`. You don't pass `--features` for tests, and the actors never reach release binaries.

7. **Write native-backend tests first**: When the native backend lands, you should be able to run your existing scenario against it by implementing `VpnBackend` alone — with zero scenario or harness changes. Design assertions with that future in mind.

## 🔗 Related Documentation

- [Feature Spec](./spec.md) - Problem, strategic intent, requirements (FR-001..FR-014), success criteria
- [Implementation Plan](./plan.md) - Architecture, project structure, constitution check
- [Data Model](./data-model.md) - Entities, `LifecycleEvent`, state machines, data flow
- [Backend & Actor Contracts](./contracts/system-effects-contract.md) - `VpnBackend`/`LifecycleEvent`/actor/harness contracts

## 🆘 Need Help?

- **What event should I assert?**: See the `LifecycleEvent` vocabulary and state machine in [data-model.md](./data-model.md)
- **What does the harness guarantee?**: See the `TestHarness` + `Timeline` contract in [contracts/system-effects-contract.md](./contracts/system-effects-contract.md)
- **Why backend-agnostic?**: See "Strategic Intent" in [spec.md](./spec.md) — this is the migration safety net for removing `openconnect`

---

**Ready to write a scenario?** Start with `ScenarioBuilder::new()`, run it through `TestHarness::new(SimulatedBackend::...)`, and assert on the `Timeline`. No root, no network, no openconnect. 🚀
