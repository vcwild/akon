---
description: "Task list for 008-vpn-on-background"
---

# Tasks: `akon vpn on` background mode and production log levels

**Branch**: `008-vpn-on-background`

## Format: `[ID] [P?] [Story] Description`
- **[P]**: can run in parallel (different files, no dependency)

---

## Phase 1: Log-level gating (Story 2 — simplest, no architecture change)

- [ ] T001 [US2] Gate all `[tun-cfg]` and `[tun-io]` `eprintln!` calls in
      `akon-core/src/vpn/f5/tun.rs` behind `debug_enabled()`. Error/WARN
      lines (containing `ERROR:` or `WARN`) stay unconditional.
- [ ] T002 [P] [US2] Gate `[dns]` trace `eprintln!` calls in
      `akon-core/src/vpn/f5/dns.rs` and `backend.rs` behind `debug_enabled()`.
      DNS warning ("failed to apply VPN DNS") stays unconditional.
- [ ] T003 [P] [US2] Write unit tests (offline, no root) asserting:
      - With `AKON_F5_DEBUG` unset, running the FakeTun/FakeDns connect flow
        produces no `[tun-cfg]`, `[tun-io]`, or `[dns]` lines in stderr.
      - With `AKON_F5_DEBUG=1`, all trace lines are present (no regression).
      (Use the existing `native_f5_dataplane_tests` harness or a new lightweight
      capture test in `akon-core/tests/`.)

**Checkpoint**: Production output is clean; all traces behind the flag. Tests green.

---

## Phase 2: Foreground flag (plumbing for background mode)

- [ ] T004 [US1] Add `--foreground` / `-f` flag to `akon vpn on` in
      `src/main.rs` (clap `On { force: bool, foreground: bool }`). Wire it
      through `run_vpn_on(force, foreground)` → `run_vpn_on_native`. When
      `foreground == true` or not Linux, behaviour is unchanged (blocking).

**Checkpoint**: `--foreground` compiles and preserves existing blocking mode.

---

## Phase 3: Background mode (Story 1)

### Tests first (TDD)

- [ ] T005 [P] [US1] Write unit tests for `ConnectResult` pipe encode/decode
      in `src/cli/background.rs`:
      - `ConnectResult::Success { ip, device }` round-trips through the pipe.
      - `ConnectResult::Failure { message }` round-trips.
      - Partial/truncated pipe read returns an error (not a hang).

### Implementation

- [ ] T006 [US1] Create `src/cli/background.rs` with:
      - `ConnectResult` enum (Success + Failure).
      - `encode` / `decode` (compact binary: 1-byte tag + u16 length + UTF-8).
      - `fork_and_connect(vpn_fn) -> Result<ConnectResult, AkonError>`:
        opens a pipe, `fork()`s (nix), child side runs `vpn_fn` (the
        connect+state-write step), writes result to pipe, detaches (`setsid`,
        redirect stdio to log file), then continues running the VPN; parent
        side reads result (30s timeout), returns it.
- [ ] T007 [US1] Wire `fork_and_connect` into `run_vpn_on` (Linux, when
      `!foreground`): call `fork_and_connect(|| native_connect_once(…))`, print
      the connected summary from the parent, exit 0 on success / exit 1 on
      failure.
- [ ] T008 [US1] Determine the log file path (`$XDG_DATA_HOME/akon/vpn.log`)
      and ensure the child creates it (with parent dirs) before redirecting
      stderr. Print `Running in background (logs: <path>)` from the parent.

---

## Phase 4: Edge cases and hardening

- [ ] T009 [US1] Verify failure path: if `native_connect_once` returns `Err`,
      the child writes `ConnectResult::Failure` and exits; the parent prints the
      error and exits 1; no orphaned background process. Add a test with a
      failing fake server.
- [ ] T010 [P] Verify `vpn status` + `vpn off` work correctly after backgrounding
      (state file written before child signals parent — verify ordering in code).
- [ ] T011 [P] Non-Linux: `fork_and_connect` is cfg-gated; `run_vpn_on_native`
      falls back to blocking mode unconditionally.

---

## Phase 5: Verification

- [ ] T012 `cargo fmt --check` + `cargo clippy --workspace --all-targets
      --features test-actors -- -D warnings` clean on 1.96.
- [ ] T013 Full CI-equivalent run: `cargo test --workspace --features test-actors`
      green (offline, no root, no hang). Manual live `akon vpn on` → prompt
      returned, `vpn status` reports connected, `vpn off` disconnects cleanly.

---

## Dependencies

T001/T002 → T003 (log tests verify gating).
T004 (flag) → T006/T007 (wire-up uses the flag).
T005 (pipe tests) → T006 (pipe impl).
T006 → T007 (fork_and_connect → vpn on).
T009/T010/T011 after T007.
T012/T013 last.

## Parallel opportunities

T001 ∥ T002 ∥ T005 (different files, no deps).
T003 after T001/T002. T009 ∥ T010 ∥ T011 after T007.
