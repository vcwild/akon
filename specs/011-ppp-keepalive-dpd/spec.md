# Feature Specification: PPP keepalive / DPD sender (prevent tunnel drops)

**Feature Branch**: `011-ppp-keepalive-dpd`
**Created**: 2026-06-22
**Status**: Draft
**Input**: User observation: the F5 VPN server drops the tunnel about every ~2.5
minutes; we want to know why and make it happen less often.

## Overview

A connected native session is torn down by the F5 appliance at a **deterministic
~149-second interval** (measured from the journal: connect → drop ≈ 2m29s, every
time). A fixed interval like that is the server's **DPD (Dead Peer Detection)**
timeout firing.

Root cause: akon only **replies** to the server's PPP `Echo-Request`; it never
**sends** its own PPP keepalives. openconnect, by contrast, proactively sends
periodic PPP `Echo-Request` (DPD) and `Discard-Request` (keepalive) frames on the
control channel (confirmed in its `ppp.c`: `KA_DPD` → Echo-Request, `KA_KEEPALIVE`
→ Discard-Request). Without client-originated keepalives the appliance considers
the peer dead after its DPD window (~150 s) and closes the tunnel.

The event-driven supervisor (spec 010) already *recovers* from these drops, but
that treats the symptom. This feature removes the cause: akon sends periodic PPP
keepalives during the data-plane phase so the server keeps the tunnel up
indefinitely.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — The tunnel stays up without periodic drops (Priority: P1)

As a user, my VPN connection stays up continuously; it is not torn down by the
server every couple of minutes. Reconnections become rare (only for genuine
network changes), not a routine ~2.5-minute event.

**Why this priority**: Routine drops cause brief connectivity blips on every
recovery, churn the assigned IP, and stress both client and server. Eliminating
them is the real fix; recovery (spec 010) is the safety net.

**Independent Test**: Drive the data-plane pump with a fake peer and assert that,
when no real traffic is flowing, akon emits a PPP keepalive (Echo-Request or
Discard-Request) at the configured interval. Reproducible offline with the test
actors (no live VPN).

**Acceptance Scenarios**:

1. **Given** an established tunnel with no user traffic, **When** the keepalive
   interval elapses, **Then** akon sends a PPP keepalive frame to the server.
2. **Given** real data is actively flowing, **When** the keepalive interval
   elapses, **Then** akon may skip the explicit keepalive (data already proves
   liveness) — matching openconnect's behaviour.
3. **Given** the server sends an `Echo-Request`, **When** received, **Then** akon
   still replies with an `Echo-Reply` (existing behaviour, unchanged).

### User Story 2 — Keepalive does not disrupt the data plane (Priority: P1)

As a user, the keepalive mechanism never interferes with normal traffic
forwarding or causes spurious disconnects.

**Why this priority**: A keepalive sender that corrupts the framing or blocks the
pump would be worse than the drops.

**Independent Test**: With the keepalive active, a data-plane round-trip still
succeeds (the existing netns round-trip test stays green); keepalive frames are
valid PPP control frames that the peer accepts.

**Acceptance Scenarios**:

1. **Given** the keepalive is enabled, **When** a data packet and a keepalive are
   both due, **Then** both are sent correctly and the round-trip still works.
2. **Given** a keepalive is sent, **When** the server responds (or ignores a
   Discard-Request), **Then** the tunnel remains up.

### Edge Cases

- The keepalive interval MUST be comfortably below the server's DPD tolerance
  (~150 s). A ~20–30 s interval (openconnect's default range) is safe.
- Sending a keepalive MUST NOT block the pump's packet forwarding (it shares the
  same transport; it must be interleaved, not serialized behind a read).
- If sending a keepalive fails (transport error), it is handled like any tunnel
  drop (the pump exits → supervisor reconnects) — no special-casing.
- IPv6/IP6CP rejection is unchanged; keepalives are LCP control frames.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: During the data-plane phase, akon MUST send a periodic PPP
  keepalive to the server (LCP `Echo-Request` as DPD and/or `Discard-Request`),
  mirroring openconnect, so the server's DPD does not expire.
- **FR-002**: The keepalive interval MUST be safely below the server's DPD
  tolerance (default ~20–30 s; well under the observed ~150 s drop).
- **FR-003**: When real outbound data is flowing, akon MAY skip the explicit
  keepalive for that interval (data already refreshes the server's liveness),
  matching openconnect.
- **FR-004**: Sending keepalives MUST NOT block or starve packet forwarding in
  either direction; it must be interleaved with the pump's I/O.
- **FR-005**: Replying to server `Echo-Request` with `Echo-Reply` MUST continue
  to work (no regression).
- **FR-006**: A keepalive send failure MUST be treated like a tunnel drop (pump
  exits, supervisor reconnects) — no silent stall.
- **FR-007**: The keepalive frames MUST be valid PPP LCP control frames
  (correct code, id, magic) accepted by the F5 peer.

### Key Entities

- **Keepalive timer**: drives periodic keepalive emission during the data plane.
- **PPP LCP control frame**: `Echo-Request` (DPD) / `Discard-Request` (keepalive)
  carrying the negotiated magic number.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A live session stays connected for ≥ 15 minutes with **no
  server-initiated drops** (vs. the prior ~2.5-minute drop cadence).
- **SC-002**: With no user traffic, akon emits a keepalive at the configured
  interval (verifiable offline against a fake peer).
- **SC-003**: The data-plane round-trip test remains green with keepalives
  enabled (no forwarding regression).
- **SC-004**: Server `Echo-Request` → `Echo-Reply` still works (no regression).
- **SC-005**: Covered by offline, deterministic tests (test actors) runnable
  under the standard CI `cargo test`.
