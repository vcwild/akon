# Implementation Plan: PPP keepalive / DPD sender

**Branch**: `011-ppp-keepalive-dpd` | **Date**: 2026-06-22 | **Spec**: [spec.md](./spec.md)

## Summary

Add a periodic PPP keepalive to the data-plane pump so the F5 appliance's DPD
timer never expires (the cause of the ~149 s drops). Implementation:

1. Add an `lcp_echo_request(id, magic, data)` builder in `ppp.rs` (mirrors the
   existing `lcp_echo_reply`).
2. In `pump_packets`, add a `tokio::time::interval` branch to the `select!` loop:
   on each tick, if no real outbound data has been sent since the last tick, send
   an LCP `Echo-Request` (encapsulated via `f5_encap(build_ncp_frame(...))`)
   carrying the negotiated magic. A send failure ends the pump (→ supervisor
   reconnects), same as any transport error.

The keepalive shares the pump's `select!`, so it never blocks forwarding.

## Technical Context

**Files**: `akon-core/src/vpn/f5/ppp.rs` (builder + test), `backend.rs`
(pump timer; pass `magic` into `pump_packets`).
**Interval**: 20 s default (openconnect's keepalive range; well under the
observed ~150 s server tolerance). A small const, not yet configurable.
**Dependencies**: none new (`tokio::time::interval`).
**Testing**: pure unit test for `lcp_echo_request` framing; an actor-level test
that the pump emits a keepalive when idle (the F5 server actor counts received
Echo-Requests); the netns round-trip test must stay green.
**Constraints**: keepalive must be a valid LCP frame with the negotiated magic;
must interleave with I/O; must not change the IPv6/IPCP behaviour.

## Constitution Check

- [x] **Security-First**: No credentials/secrets involved. Keepalives are LCP
  control frames; no data exposure.
- [x] **Modular Architecture**: The frame builder lives in `ppp.rs` (pure); the
  timer lives in the pump. Single responsibilities preserved.
- [x] **Test-Driven Development**: Unit test for the builder first; actor test
  for "idle pump emits keepalive".
- [x] **Observability**: Keepalive sends are logged under the existing debug gate
  (`[f5-data] keepalive`), not at production level (avoid noise).
- [x] **CLI-First Interface**: No CLI change.
- [x] **Test Actors & Seam-Isolated Testing**: The keepalive is exercised offline
  via `MemoryTransport` + the F5 server actor (which can observe Echo-Requests);
  no live VPN, no hang (bounded test with a short interval). The netns round-trip
  remains the integration check.

**Security-Critical Changes**: none.

## Design Decisions

1. **Echo-Request as the keepalive (DPD).** openconnect sends both Echo-Request
   (KA_DPD) and Discard-Request (KA_KEEPALIVE). For F5, the Echo-Request is the
   meaningful DPD (the server may reply with Echo-Reply, and our sending it
   refreshes the server's peer-liveness). We send `Echo-Request` carrying the
   magic; we do NOT require a reply to consider ourselves alive (the server-side
   DPD is what we're feeding). Optionally also send Discard-Request — start with
   Echo-Request (simplest, matches the DPD semantic) and revisit if the appliance
   needs Discard.

2. **Skip when data is flowing.** Track an `out_pkts` counter (already present);
   if it advanced since the last keepalive tick, skip the explicit keepalive that
   interval (data already refreshed liveness) — mirrors openconnect.

3. **Interval 20 s (const).** Comfortably below ~150 s. Hardcoded for now; a
   config knob can come later if needed.

4. **Failure = drop.** If the keepalive `transport.send` errors, return
   `ServerClosed` from the pump (same as a normal transport failure) so the
   event-driven supervisor reconnects.

## Project Structure

```
akon-core/src/vpn/f5/
├── ppp.rs       # ADD lcp_echo_request(id, magic, data) + unit test
└── backend.rs   # pump_packets: add keepalive interval branch; pass `magic`
                 #   from Session into pump_packets

akon-core/src/vpn/testkit/f5_server_actor.rs  # (if needed) observe Echo-Requests
akon-core/tests/ (or inline)                  # actor test: idle pump emits keepalive
```

## Complexity Tracking

No constitution violations. Small, well-scoped protocol-conformance addition that
mirrors the reference implementation (openconnect).
