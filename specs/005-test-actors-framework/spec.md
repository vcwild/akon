# Feature Specification: Test Actors Framework

**Feature Branch**: `005-test-actors-framework`
**Created**: 2026-06-21
**Status**: Draft
**Input**: User description: "Implement a test actors framework on top of the akon VPN tool to test its functionalities without losing access to the internet. We want the project to work reliably in real-world scenarios but we fail to emulate them because we need real connectivity. Implement the test framework and a few tests to test its functionality."

## Problem Statement

akon orchestrates a real `openconnect` child process via `sudo`, discovers its PID with `pgrep`/`ps`, signals it with `kill`, and verifies connectivity with real HTTP(S) health checks. Every one of these touches the live operating system and the live network. As a result, the most important real-world behaviors — successful connect, authentication failure, silent tunnel death, suspend/resume, flaky networks, automatic reconnection — **cannot be exercised in automated tests** without root privileges, a real VPN endpoint, and a connection that would disrupt the developer's own internet access.

The project needs a way to **emulate real-world scenarios deterministically and offline**, so the connect/disconnect/reconnect logic can be tested reliably without losing internet access or requiring privileged infrastructure.

## Strategic Intent (why this framework matters beyond testing)

This framework is the **migration safety net for removing the `openconnect` dependency**. akon's long-term goal is to replace the `openconnect`-delegation mechanism with its own native VPN implementation (no external process, no required dependencies). That replacement is high-risk: it reimplements the handshake, session, tunnel, and teardown that openconnect handles today.

To make that migration safe, akon needs a **backend-agnostic scenario suite**: the same real-world scenarios must validate the *current* openconnect-delegating backend AND a *future* native backend, and both must produce identical observable behavior. Therefore the framework's abstraction boundary MUST be defined in terms of **VPN connection behavior akon will still own after openconnect is gone** (connection lifecycle, tunnel/link state, health), NOT in terms of openconnect-specific artifacts (child process, stdout lines, `pgrep`/`kill`). Openconnect-specific handling becomes an implementation detail of one backend; the simulated actors model the network/server/transport reality that any backend must satisfy.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Simulate a VPN connection lifecycle offline (Priority: P1)

As an akon developer, I can drive the full connection lifecycle (connect → authenticate → session established → tunnel/link up → connected → disconnect) against a simulated backend instead of a real `openconnect` process, so I can assert akon reacts correctly without root, without a real server, and without touching my network.

**Why this priority**: This is the core of the framework. Without an offline substitute for the VPN backend and its OS effects, no real-world scenario can be tested at all. It is the MVP — it delivers value on its own by making the happy-path connect flow testable. The lifecycle is expressed in **backend-agnostic terms** so the same test will later validate a native backend.

**Independent Test**: Build a scenario where a simulated VPN backend emits a scripted, successful connection lifecycle; run akon's connection logic against it; assert that the observed lifecycle events end in `Connected` with the expected IP/device and that the simulated tunnel/process is "alive".

**Acceptance Scenarios**:

1. **Given** a scripted successful backend, **When** akon connects through the test harness, **Then** the observed lifecycle ends in `Connected { ip, device }` and the harness reports the simulated tunnel as alive with a known handle/PID.
2. **Given** an established simulated connection, **When** the developer issues disconnect through the harness, **Then** the simulated backend tears down (receives the terminate signal, transitions to terminated) and no real `kill`/`pgrep`/process call is invoked.
3. **Given** a backend scripted to emit an authentication failure, **When** akon connects, **Then** the flow ends in an error (no `Connected` event) and the simulated tunnel is not left alive.

---

### User Story 2 - Emulate network interruptions and verify reconnection (Priority: P2)

As an akon developer, I can control connectivity state (reachable / unreachable / flaky) via a network actor so the health-check + reconnection logic reacts the same way it would on a real flaky Wi-Fi, without a real endpoint and without affecting my actual internet.

**Why this priority**: Reconnection is the project's reliability promise. It is currently only testable against `192.0.2.1` (guaranteed-fail) or a real server. A controllable network actor lets us reproduce silent tunnel death and recovery deterministically. It builds on US1 but is independently valuable.

**Independent Test**: Drive a network actor from "up" to "down" for N polls then back to "up"; assert the reconnection manager observes the failures, triggers a reconnect, and returns to a connected/healthy state.

**Acceptance Scenarios**:

1. **Given** a network actor reporting "up", **When** health checks run, **Then** every check succeeds and no reconnection is triggered.
2. **Given** a network actor that goes "down" for a configured number of polls, **When** the failure threshold is reached, **Then** a reconnection attempt is triggered.
3. **Given** a network actor that recovers to "up" after going down, **When** the reconnect attempt runs, **Then** the connection returns to a healthy state.

---

### User Story 3 - Author scenarios declaratively and assert on observed behavior (Priority: P3)

As an akon developer, I can describe a real-world scenario as data (a scripted sequence of server/network/tunnel events with timing) using a small builder API and run it through a single harness entry point, so writing a new real-world regression test is quick and readable.

**Why this priority**: Ergonomics. The raw actors (US1/US2) are enough to write tests, but a declarative scenario builder and a recording harness make scenarios self-documenting and lower the cost of adding new real-world regression tests. It is a usability layer on top of the MVP.

**Independent Test**: Use the builder to compose a multi-step scenario (connect, run healthy, drop network, reconnect), run it, and read back a recorded timeline of observed events to assert ordering.

**Acceptance Scenarios**:

1. **Given** a scenario authored via the builder, **When** it is run through the harness, **Then** the harness returns a recorded, ordered timeline of observed events.
2. **Given** a recorded timeline, **When** the developer asserts a sub-sequence of events occurred in order, **Then** the assertion helper passes for matching timelines and fails with a clear message otherwise.

---

### User Story 4 - Validate the same scenarios against a swappable backend (Priority: P2)

As an akon developer planning to replace `openconnect` with a native implementation, I can run the **same** scenario suite against any backend that implements akon's connection boundary, so that when I introduce a native backend I can prove it behaves identically to the openconnect backend before switching the default.

**Why this priority**: This is the strategic payoff — the framework's reason for existing beyond unit testing. It is P2 (not P1) because it depends on the backend boundary and harness from US1, but it is what makes the openconnect-removal migration safe. It must be in place *before* a native backend is written, so the native backend can be developed test-first.

**Independent Test**: Define the connection boundary as a trait; run an identical scenario (e.g., connect → healthy → drop → reconnect) twice — once against the simulated backend and once against an adapter wrapping the existing openconnect path — and assert both produce the same observable lifecycle timeline.

**Acceptance Scenarios**:

1. **Given** a backend-agnostic scenario, **When** it is run against the simulated backend, **Then** it produces a lifecycle timeline that conforms to the connection boundary contract.
2. **Given** the same scenario, **When** it is run against a different backend implementing the same boundary, **Then** the observable lifecycle timeline is equivalent (same ordered lifecycle states), demonstrating backend swappability.
3. **Given** the connection boundary trait, **When** a new (e.g., future native) backend is added, **Then** no scenario or harness code must change for it to be exercised by the existing suite.

### Edge Cases

- What happens when the server actor's script is exhausted before a terminal event (connect/error)? The harness MUST surface a deterministic timeout/"script exhausted" outcome rather than hanging.
- What happens when disconnect is requested for a simulated process that already terminated? It MUST be a no-op success (mirrors real behavior).
- What happens when a scenario requests reconnection but the network actor never recovers? The reconnection MUST exhaust its retry policy and report a terminal failure deterministically.
- How does the framework guarantee it never reaches the real OS or network? The real system-effects implementation MUST NOT be reachable from harness-driven tests; tests use only the simulated implementation.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST define a **backend-agnostic connection boundary** — an abstraction over "establish a VPN connection, report its lifecycle, and tear it down" — expressed in terms akon will still own after `openconnect` is removed (connection lifecycle events, tunnel/link state, health), NOT in openconnect-specific terms (child process, stdout text, `pgrep`/`kill`).
- **FR-002**: The system MUST retain a real backend whose behavior is equivalent to today's production code path (delegating to `openconnect`), so production connectivity is unchanged. Openconnect-specific operations (spawn via `sudo`, `pgrep`/`ps`, `kill`, stdout parsing) MUST be confined to this backend as an implementation detail behind the boundary.
- **FR-003**: The system MUST provide a simulated backend implementing the same connection boundary, backed by in-memory actors, requiring no root, no real process, and no network.
- **FR-004**: The simulated VPN server actor MUST be scriptable to emit an ordered, backend-agnostic connection lifecycle (connecting, authenticating, session established, tunnel/link up, connected, errors, disconnect) that any backend must be able to produce.
- **FR-005**: The simulated process/tunnel registry MUST track simulated connection handles, report liveness, and respond to termination signals (graceful then forced) with realistic state transitions — without ever invoking real OS process calls.
- **FR-006**: The system MUST provide a network actor that controls health-check reachability (reachable / unreachable / scripted per-poll), so reconnection logic can be exercised offline.
- **FR-007**: The system MUST provide a test harness that wires a chosen backend + actors together, runs a scenario, and records an ordered timeline of observed lifecycle events and state transitions.
- **FR-008**: The system MUST provide a scenario builder API to compose real-world scenarios (e.g., connect, stay healthy, drop network, reconnect, fail auth) as readable, **backend-independent** test data.
- **FR-009**: The framework MUST guarantee that harness-driven tests never reach the real OS or network (no real `sudo`/`openconnect`/`pgrep`/`kill`, no real HTTP requests) when using the simulated backend.
- **FR-010**: The framework MUST be available to tests without enabling it in production builds (gated behind a test/feature seam), so it adds no runtime cost or attack surface to released binaries.
- **FR-011**: The framework MUST provide assertion helpers to verify that an expected ordered sub-sequence of lifecycle events occurred in a recorded timeline, with clear failure messages.
- **FR-012**: At least the following real-world scenarios MUST be demonstrated by tests using the framework: (a) successful connect + clean disconnect, (b) authentication failure, (c) network interruption followed by successful reconnection.
- **FR-013**: The connection boundary MUST be designed so that a future native backend (no external dependencies) can be added by implementing the same trait, with **no changes to scenarios or the harness** — enabling the eventual removal of the `openconnect` delegation to be developed and validated test-first.
- **FR-014**: The framework MUST make it possible to run the **same** scenario against more than one backend and compare the observable lifecycle timelines for equivalence, so a replacement backend can be proven behaviorally equivalent before becoming the default.

### Key Entities

- **Connection Boundary (Backend trait)**: The backend-agnostic interface over establishing/observing/tearing down a VPN connection. Implemented by the openconnect backend today and a future native backend tomorrow; the simulated backend implements it for tests.
- **VPN Server Actor**: An in-memory actor that plays the role of the remote VPN server, driven by a script of backend-agnostic lifecycle outcomes/timings.
- **Tunnel/Process Registry (Fake)**: In-memory store of simulated connection handles, with liveness and signal-handling (graceful/forced teardown) semantics. Models what any backend must track, not openconnect PIDs specifically.
- **Network Actor**: Controls simulated connectivity/health-check reachability over time.
- **Scenario**: Declarative, backend-independent description of a real-world situation (sequence of actor events + timing) used to drive a test against any backend.
- **Harness / Recorder**: Orchestrates a backend + actors for a scenario and produces an ordered, assertable timeline of observed lifecycle events and state transitions.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can run the new framework tests with a plain `cargo test` on a machine with **no VPN server, no root privileges, and no special network setup**, and they pass.
- **SC-002**: Running the framework tests does **not** alter the host's network connectivity or routing in any way (the developer keeps full internet access throughout).
- **SC-003**: The three mandated real-world scenarios (successful connect+disconnect, auth failure, interruption+reconnect) are each covered by at least one passing automated test.
- **SC-004**: No harness-driven test invokes a real `sudo`, `openconnect`, `pgrep`, `ps`, or `kill`, and no real outbound HTTP request is made (verifiable by the simulated boundary being the only one wired in).
- **SC-005**: Adding a new real-world scenario test requires only composing a scenario via the builder and asserting on the recorded timeline — no changes to production source modules.
- **SC-006**: The released (non-test) build is unchanged in behavior and contains no test-actor code paths reachable at runtime.
- **SC-007**: A new backend can be exercised by the existing scenario suite by implementing the connection boundary trait alone — no scenario or harness code changes required (verifiable by the simulated backend and an openconnect-adapter backend both running the same scenario).
- **SC-008**: The connection lifecycle observed by tests is expressed entirely in backend-agnostic terms; no scenario or assertion references openconnect-specific artifacts (process IDs from `pgrep`, stdout strings, `sudo`), so the suite remains valid after `openconnect` is removed.
