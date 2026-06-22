---
description: "Task list for 010-robust-reconnection-fresh"
---

# Tasks: Robust reconnection (fresh OTP; no stale tunnel)

**Branch**: `010-robust-reconnection-fresh`

## Phase 1: Credential selection (the root-cause fix)

- [x] T001 [US1] Add a pure `select_password(initial, env_password, gen)` helper
      in `src/cli/vpn.rs`: initial → env value if present, else generate;
      reconnect (initial=false) → always generate fresh.
- [x] T002 [P] [US1] Unit tests for `select_password`:
      - initial + env present → returns env value
      - initial + env empty/absent → calls gen
      - reconnect (initial=false) + env present → IGNORES env, calls gen
      - gen error propagates

## Phase 2: Wire it through the connect/supervise paths

- [x] T003 [US1] Add `initial: bool` to `native_connect_once`; replace the
      direct `AKON_VPN_PASSWORD` read with `select_password(initial, env, || generate_password(...))`.
- [x] T004 [US1] `run_vpn_on_native`: call `native_connect_once(.., initial = true)`.
- [x] T005 [US1] `native_supervise`: reconnect loop calls
      `native_connect_once(.., initial = false)` → fresh OTP each attempt.

## Phase 3: Fail-safe exhaustion (US2)

- [x] T006 [US2] On exhausting `max_attempts`, ensure the backend is
      disconnected/reconciled and the state file no longer claims connected
      (honest status; no half-tunnel). Verify the data-plane drop already
      reconciles; add explicit cleanup + clear message if not.

## Phase 4: Verification

- [x] T007 fmt + clippy (1.96) clean; full CI-equivalent
      `cargo test --workspace --features test-actors` green.
- [x] T008 Manual/gated: a live session stays up ≥ 10 minutes (past the OTP
      window), surviving at least one reconnect. (Not part of CI.)

## Dependencies
T001 → T002 (tests) → T003 → T004/T005 → T006 → T007. T008 last (manual).
