---
description: "Task list for 007-fix-vpn-status"
---

# Tasks: Reliable `akon vpn status` for the native backend

**Input**: spec.md, plan.md, data-model.md, contracts/status-contract.md
**Branch**: `007-fix-vpn-status`

## Format: `[ID] [P?] [Story] Description`
- **[P]**: can run in parallel (different files, no dependency)

## Path Conventions
- Library: `akon-core/src/vpn/...`
- CLI: `src/cli/vpn.rs`

---

## Phase 1: Foundational (ground-truth adapter)

- [x] T001 Add privilege-free `interface_exists(name) -> bool` and
      `interface_ipv4(name) -> Option<Ipv4Addr>` to
      `akon-core/src/vpn/f5/netlink.rs` (via `if_nametoindex` / `getifaddrs`).
- [x] T002 [P] Add a bounded real adapter test in `netlink.rs` `#[cfg(test)]`:
      `interface_exists("lo")` is true; a clearly-absent name is false;
      `interface_ipv4("lo")` is `Some(127.0.0.1)` (loopback always exists). No
      root, no hang.

**Checkpoint**: ground-truth lookup available and verified.

---

## Phase 2: User Story 1 + 2 + 3 — status reconciliation (P1/P1/P2) 🎯 MVP

These stories share one decision function and one command; implemented together.

### Tests first (TDD)

- [x] T003 [US1/US2/US3] Add unit tests for a pure `evaluate_status(record,
      interface_present, live_ip) -> StatusVerdict` in `src/cli/vpn.rs`
      `#[cfg(test)]`:
  - no record → `NotConnected` (exit-code 1 mapping)
  - record + interface present → `Connected` with live IP preferred, recorded IP
    fallback, and `since` from `connected_at`
  - record + interface absent → `Stale("tunnel interface no longer present")`
  - record without `device` → `Stale("no tunnel device recorded")`
  - **PID-independence**: interface present + dead PID → still `Connected`;
    interface absent + alive PID → `Stale`

### Implementation

- [x] T004 [US1/US2/US3] Introduce a `StatusVerdict` enum + pure
      `evaluate_status(...)` in `src/cli/vpn.rs` (no I/O), mapping per
      data-model.md.
- [x] T005 [US1/US2/US3] Rewrite `run_vpn_status()` to: read+parse the record
      (robust to missing/corrupt), query the interface via the Phase-1 adapter,
      call `evaluate_status`, then render per contracts/status-contract.md and
      exit with stable codes (0/1/2). Remove the PID-as-truth check; keep the PID
      line advisory with a "(not running)" suffix when the owner is dead.
- [x] T006 [US2] Confirm `run_vpn_off()` clears the record so a subsequent status
      returns `NotConnected` (no code change expected; add/confirm coverage).

**Checkpoint**: status reports Connected/Stale/NotConnected correctly,
independent of PID.

---

## Phase 3: Edge cases & hardening

- [x] T007 Ensure non-Linux builds compile (interface lookup → false) and status
      degrades gracefully (cfg-gated adapter wrappers in `src/cli/vpn.rs`).
- [x] T008 [P] Robustness test: corrupt/partial state file → clear error, no
      panic; missing `device` → Stale.

---

## Phase 4: Verification (polish)

- [x] T009 `cargo fmt --check`, `cargo clippy --workspace --all-targets
      --features test-actors -- -D warnings` clean on the CI toolchain (1.96).
- [x] T010 Full CI-equivalent run green: `cargo test --workspace --features
      test-actors` (no root, no live VPN). Manual live check per quickstart.md.

---

## Dependencies

- T001 blocks T005 (status uses the adapter).
- T003 (tests) precede T004/T005 (TDD).
- T004 blocks T005 (status calls `evaluate_status`).
- T007/T008 after T005. T009/T010 last.

## Parallel opportunities

- T002 ∥ T003 (different files: netlink tests vs. cli tests).
- T008 ∥ T007 once T005 lands.
