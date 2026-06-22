# Feature Specification: Robust reconnection (fresh OTP; no stale tunnel)

**Feature Branch**: `010-robust-reconnection-fresh`
**Created**: 2026-06-22
**Status**: Draft
**Input**: User report: "akon vpn on works for some time but about 3 minutes
after we get a stale tunnel interface."

## Overview

A connected native VPN session goes **stale after ~3 minutes**: the tunnel
interface disappears and `akon vpn status` reports "inactive (stale)".

Root cause (two compounding bugs in the in-process supervisor):

1. **Stale one-time OTP reused on reconnect.** In background mode the parent
   generates a PIN+OTP once and passes it to the child via the
   `AKON_VPN_PASSWORD` environment variable. The child reuses that **same** value
   for every reconnection attempt. TOTP codes expire within 30–60 s, so any
   reconnect minutes later authenticates with an **expired OTP** and fails.

2. **A failed reconnect leaves no tunnel.** The supervisor calls
   `disconnect()` (which removes the tun interface) *before* attempting to
   reconnect. With `health_check_interval_secs = 60` and
   `consecutive_failures_threshold = 1`, a single transient health-check failure
   around the 2nd–3rd interval (~120–180 s ≈ **3 minutes**) tears down the
   working tunnel; the reconnect then fails (bug 1) and the user is left with a
   **stale, gone interface**.

The fix: always authenticate reconnects with a **freshly generated** OTP (never
reuse the one-shot env value), and make the supervisor resilient so a transient
health blip or a single failed attempt cannot strand the user with a dead tunnel.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Stays connected past the OTP window (Priority: P1)

As a user, my VPN stays connected indefinitely; if the supervisor ever needs to
reconnect, it authenticates successfully with a fresh OTP rather than failing on
an expired one.

**Why this priority**: This is the reported defect — the connection dies after a
few minutes. A VPN that drops itself is unusable.

**Independent Test**: Drive the supervisor's reconnect path and assert it
generates a NEW credential for the reconnect attempt (does not reuse a stale
one-shot value). Reproducible offline by injecting a credential source and
asserting it is re-invoked per attempt.

**Acceptance Scenarios**:

1. **Given** a connected session whose initial OTP has since expired, **When**
   the supervisor reconnects, **Then** it generates a fresh OTP and the reconnect
   authenticates successfully.
2. **Given** background mode (OTP passed via env at start), **When** a reconnect
   happens minutes later, **Then** the reconnect does NOT reuse the env OTP — it
   uses a freshly generated one.

### User Story 2 — A transient blip never strands the tunnel (Priority: P1)

As a user, a momentary health-check failure (a single timeout / DNS blip / a 4xx)
does not destroy my working VPN. If a reconnect is genuinely needed and an
attempt fails, the supervisor keeps the tunnel intact / keeps trying rather than
leaving me disconnected.

**Why this priority**: Even with the OTP fix, a too-eager teardown plus a single
failed attempt should not leave a stale interface. Reconnection must fail safe.

**Independent Test**: Simulate health-check failures and a failing first
reconnect attempt; assert the supervisor does not end in a permanently stale
state (it retries per policy and/or does not tear down until it can replace the
tunnel).

**Acceptance Scenarios**:

1. **Given** a single health-check failure followed by recovery, **When** the
   supervisor evaluates health, **Then** it does not tear down a still-working
   tunnel unnecessarily (subject to the configured threshold).
2. **Given** a reconnect is triggered and the first attempt fails, **When**
   subsequent attempts run per policy, **Then** the session recovers without
   manual intervention, OR the user is clearly informed it is retrying (not
   silently left stale).
3. **Given** all reconnect attempts are exhausted, **When** the supervisor gives
   up, **Then** the host is left in a clean, reconciled state (no half-configured
   tunnel) and status reflects it honestly.

### Edge Cases

- The initial `AKON_VPN_PASSWORD` env OTP must still work for the **first**
  connect (it is fresh then); only **reconnects** must regenerate.
- Keyring unavailable at reconnect time → reconnect fails gracefully and retries
  per policy; no panic.
- Reconnect that reaches `Connected` must leave a healthy tunnel + correct state
  file (so `status` reports connected, `off` tears down cleanly).
- Foreground mode and background mode must behave identically for reconnection.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Reconnection attempts MUST authenticate with a **freshly generated**
  PIN+OTP, never a reused one-time value. The `AKON_VPN_PASSWORD` env value MUST
  be used for the **initial** connect only, not for reconnects.
- **FR-002**: A successful reconnect MUST result in a working tunnel and an
  updated, accurate session record (status = connected; the interface exists).
- **FR-003**: A transient, recoverable health-check failure MUST NOT be treated
  as a permanent failure that strands the user (honoring the configured
  `consecutive_failures_threshold`, which is already the intended knob).
- **FR-004**: If a reconnect attempt fails, the supervisor MUST continue
  attempting per the reconnection policy (up to `max_attempts` with backoff)
  rather than stopping after one failure.
- **FR-005**: If all reconnect attempts are exhausted, the host MUST be left in a
  clean reconciled state (no leaked/half-configured tunnel) and the failure MUST
  be observable (state + logs), not a silent stale interface.
- **FR-006**: The credential regeneration MUST read from the keyring (the
  rootless path runs as the user with keyring access); it MUST NOT log secrets.
- **FR-007**: Behaviour MUST be identical between foreground and background modes.

### Key Entities

- **Credential source**: produces a fresh PIN+OTP on demand. The initial connect
  may use the pre-supplied env value; reconnects always request a fresh value.
- **Supervisor**: the in-process health-check + reconnection loop.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A native VPN session stays connected well beyond the OTP window
  (≥ 10 minutes) without going stale, including across at least one reconnect.
- **SC-002**: A reconnect triggered after the initial OTP has expired
  authenticates successfully (fresh OTP).
- **SC-003**: A single transient health-check failure does not produce a stale
  tunnel.
- **SC-004**: When reconnection is exhausted, no half-configured tunnel remains
  and status/logs reflect the failure honestly.
- **SC-005**: Covered by offline, deterministic tests (no live VPN) that exercise
  the reconnect credential path and the fail-safe behaviour, runnable under the
  standard CI `cargo test`.
