# Implementation Plan: Reliable `akon vpn status` for the native backend

**Branch**: `007-fix-vpn-status` | **Date**: 2026-06-21 | **Spec**: [spec.md](./spec.md)

## Summary

Make `akon vpn status` reflect reality for the native backend by deriving
"connected" from the **ground-truth existence of the session's TUN interface**
(and reading the live IP from it) instead of trusting a recorded PID. The
connection lifecycle is modeled by the existing `ConnectionState` state machine
(`akon-core/src/vpn/state.rs`); the persisted session record is a snapshot of
that state, and `status` reconciles the snapshot against the live interface. The
interface-presence check is placed behind a small seam so status logic is unit-
tested offline (interface present/absent) without a live VPN.

## Technical Context

**Language/Version**: Rust 2021, MSRV 1.70 (local + CI toolchain 1.96)
**Primary Dependencies**: existing only — `libc` (for `if_nametoindex`/`getifaddrs`),
`serde`/`serde_json` (state record), `chrono` (uptime). No new dependencies.
**Storage**: the existing VPN state file (`/tmp/akon_vpn_state.json`, overridable
via `AKON_STATE_FILE`).
**Testing**: `cargo test`; offline unit tests over a seam that simulates interface
presence/absence and a parsed state record.
**Target Platform**: Linux (TUN); graceful no-op on non-Linux.
**Project Type**: single (akon CLI + akon-core library).
**Performance Goals**: status returns in ~1s; no privilege escalation.
**Constraints**: stable exit codes (0 connected, 1 not connected, 2 stale); no
sudo for `status`; no panics on missing/corrupt state.
**Scale/Scope**: one CLI command + one or two small library helpers.

## Constitution Check

*GATE: passed (re-checked after design).*

- [x] **Security-First**: No credentials touched. Status reads only public
  metadata (device, IP, server, connect time, PID) and the kernel interface list.
  No secrets in code/logs.
- [x] **Modular Architecture**: Interface-presence/IP lookup lives in akon-core
  (`vpn/f5/netlink`) with a single responsibility; the CLI consumes it. Status
  decision logic is a pure function over (state record, interface-present) so it
  has a clear boundary.
- [x] **Test-Driven Development**: Write failing unit tests for the status
  decision first (connected / not-connected / stale, PID-independent), then
  implement. Not a security-critical module (>90% rule N/A), but the decision
  function gets full case coverage.
- [x] **Observability**: Status is read-only; no state changes. Existing logging
  unaffected. No secrets logged.
- [x] **CLI-First Interface**: This *is* a CLI command; output stays
  human-readable with stable, scriptable exit codes.
- [x] **Test Actors & Seam-Isolated Testing**: The real OS integration (does a
  tun interface exist?) is the "heavy" dependency. The **decision logic** is
  factored into a pure function `evaluate_status(record, interface_present,
  interface_ip)` so it is tested offline/deterministically with simulated inputs
  (no root, no live VPN, no hang). The thin adapter that actually queries the
  kernel (`interface_exists`/`interface_ipv4`) is exercised by a bounded real
  check (e.g. against loopback `lo`, which always exists) — the production seam.
  No test-only code ships in release builds.

**Security-Critical Changes**: none (no auth/OTP/keyring/password/secret-config
paths touched).

## Project Structure

### Documentation (this feature)

```
specs/007-fix-vpn-status/
├── spec.md
├── plan.md          # this file
├── data-model.md    # Phase 1 (status decision states + record fields)
├── quickstart.md    # Phase 1 (how to verify)
├── contracts/
│   └── status-contract.md   # exit codes + output contract
└── tasks.md         # Phase 2 (/speckit.tasks)
```

### Source Code (repository root)

```
akon-core/src/vpn/
├── state.rs                 # existing ConnectionState state machine (reused)
└── f5/netlink.rs            # ADD: interface_exists(), interface_ipv4() (privilege-free)

src/cli/
└── vpn.rs                   # MODIFY: run_vpn_status() to reconcile record vs.
                             #         interface; ADD pure evaluate_status() + tests
```

**Structure Decision**: Single project (existing layout). The ground-truth
lookup is a small additive helper in the existing `netlink` module (already the
home of `if_nametoindex`); the status command is refactored in place. The
decision is isolated into a pure function for offline testing.

## Design Decisions

1. **Ground truth = TUN interface, not PID.** "Connected" iff the session's
   recorded `device` exists as a kernel interface. The IP shown is read **live**
   from that interface (fallback to the recorded IP if unreadable). The PID
   becomes advisory (we may note "owner not running" but it never flips the
   verdict).
2. **State machine snapshot.** The persisted record is a serialization of
   `ConnectionState` metadata (already used: server/device/ip/connected_at/pid +
   teardown_plan). `status` maps (record + interface_present) → a displayed state
   (Connected / Stale / Disconnected). No new persistence format is required
   beyond what `vpn on` already writes; status just stops trusting the PID.
3. **Pure decision function.** `evaluate_status(record, present, live_ip) ->
   StatusVerdict` is pure and unit-tested; the I/O (read file, query kernel,
   print) stays at the edges.
4. **Privilege-free.** `if_nametoindex`/`getifaddrs` need no `CAP_NET_ADMIN`, so
   `status` never prompts for sudo (satisfies FR-009).

## Complexity Tracking

No constitution violations; no complexity deviations to justify.
