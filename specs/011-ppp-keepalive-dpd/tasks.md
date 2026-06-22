---
description: "Task list for 011-ppp-keepalive-dpd"
---

# Tasks: PPP keepalive / DPD sender

**Branch**: `011-ppp-keepalive-dpd`

## Phase 1: LCP Echo-Request builder

- [ ] T001 [US1] Add `lcp_echo_request(id: u8, magic: u32, data: &[u8]) ->
      NcpPacket` to `akon-core/src/vpn/f5/ppp.rs` (mirror `lcp_echo_reply`,
      code = ECHOREQ).
- [ ] T002 [P] [US1] Unit test: `lcp_echo_request` produces a valid LCP
      Echo-Request frame (code 9, carries the magic) that round-trips through
      `build_ncp_frame`/`parse_ppp_frame`.

## Phase 2: Periodic keepalive in the pump

- [ ] T003 [US1] Pass the negotiated `magic` into `pump_packets` (from
      `Session.magic` via `run_data_plane`).
- [ ] T004 [US1] Add a `tokio::time::interval` (20 s) branch to the pump's
      `select!`. On tick: if `out_pkts` advanced since the last tick, skip
      (data is flowing); else build `lcp_echo_request(id, magic, &magic.to_be_bytes())`,
      `f5_encap(build_ncp_frame(..))`, and `transport.send`. On send error,
      return `DisconnectReason::ServerClosed`. Log under the debug gate.
- [ ] T005 [US2] Ensure the keepalive shares the select (does not block
      forwarding); the id increments per keepalive.

## Phase 3: Tests

- [ ] T006 [P] [US1] Actor test: wire the native backend to the F5 server actor
      over MemoryTransport with a SHORT keepalive interval (test hook or a small
      const override), drive to Connected, and assert the server actor observes
      at least one client Echo-Request within a bounded time while idle.
- [ ] T007 [US2] Confirm `native_f5_netns_roundtrip_tests` (data-plane round
      trip) stays green with keepalives enabled.

## Phase 4: Verification

- [ ] T008 fmt + clippy (1.96) clean; full CI-equivalent suite green.
- [ ] T009 Manual: live session stays up ≥ 15 min with NO server-initiated drop
      (vs prior ~2.5 min cadence). (Not part of CI.)

## Dependencies
T001 → T002 → T003 → T004 → T005. T006 after T004. T008/T009 last.
