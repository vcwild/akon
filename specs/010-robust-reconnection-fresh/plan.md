# Implementation Plan: Robust reconnection (fresh OTP; no stale tunnel)

**Branch**: `010-robust-reconnection-fresh` | **Date**: 2026-06-22 | **Spec**: [spec.md](./spec.md)

## Summary

Fix the ~3-minute stale-tunnel bug. Two changes in the CLI supervisor
(`src/cli/vpn.rs`):

1. **Fresh OTP on reconnect (the root cause).** `native_connect_once` currently
   always prefers `AKON_VPN_PASSWORD` (a one-time OTP the parent passed at start).
   Reconnects minutes later reuse that **expired** OTP and fail. Fix: the env OTP
   is consumed for the **initial** connect only; reconnect attempts regenerate a
   fresh PIN+OTP from the keyring.

2. **Fail-safe reconnection.** A reconnect that fails must not leave the user
   permanently stale: keep retrying per policy (already coded), and ensure the
   final state on exhaustion is cleanly reconciled (no half-tunnel) and honestly
   reported. (The `consecutive_failures_threshold` already lets the user avoid
   tearing down on a single blip; we keep that knob and make the credential path
   correct so legitimate reconnects succeed.)

## Technical Context

**Files**: `src/cli/vpn.rs` (`native_connect_once`, `native_supervise`).
**Dependencies**: none new. Credential regen uses the existing
`generate_password(&config.username)`.
**Testing**: factor the credential decision into a pure, testable helper
(`select_password(initial: bool, env, keyring_fn)`) so the "initial uses env,
reconnect regenerates" rule is unit-tested offline without a keyring or VPN.
**Constraints**: must not log secrets; identical foreground/background behaviour.

## Constitution Check

- [x] **Security-First**: Credentials still come only from the keyring (or the
  initial env hand-off). The fix REMOVES a latent issue (reusing a stale OTP).
  No secrets logged. The pure credential-selection helper takes a closure for
  keyring access so tests never touch real secrets.
- [x] **Modular Architecture**: A small `select_password` helper isolates the
  decision; `native_supervise`/`native_connect_once` keep single responsibilities.
- [x] **Test-Driven Development**: Write the `select_password` unit tests first
  (initial→env, reconnect→regenerate, env-absent→regenerate), then implement.
- [x] **Observability**: Reconnect attempts and exhaustion are logged (no
  secrets). On exhaustion, state is reconciled and the failure is visible.
- [x] **CLI-First Interface**: No CLI surface change; behaviour fix only.
- [x] **Test Actors & Seam-Isolated Testing**: The credential source is injected
  (closure / small trait) so the reconnect-uses-fresh-OTP rule is verified
  offline, deterministically, no keyring, no network, no hang. The live behaviour
  (10-min soak) is a manual/gated check, not part of CI.

**Security-Critical Changes**:
- [x] Password transmission / OTP generation: this touches the credential path —
  reviewed. The change makes reconnects regenerate rather than reuse; it does not
  alter how OTP is generated or transmitted.

## Design Decisions

1. **`AKON_VPN_PASSWORD` is initial-only.** `native_connect_once` gains an
   `initial: bool` (or the supervisor passes a credential provider). On the first
   connect, use the env value if present (it is fresh — the parent just made it).
   On every reconnect, ignore the env and call `generate_password` for a fresh
   OTP. This is the minimal, correct fix for the root cause.

   Implementation shape:
   ```rust
   fn select_password(
       initial: bool,
       env_password: Option<String>,
       gen: impl FnOnce() -> Result<String, AkonError>,
   ) -> Result<String, AkonError> {
       if initial {
           if let Some(p) = env_password.filter(|p| !p.trim().is_empty()) {
               return Ok(p);
           }
       }
       gen() // reconnect, or no env: always fresh
   }
   ```
   `native_connect_once(config, state_path, initial)` calls it; the first call
   from `run_vpn_on_native` passes `initial = true`, the supervisor's reconnect
   calls pass `initial = false`.

2. **Keep the policy knobs.** We do not change the user's
   `consecutive_failures_threshold`/interval. The bug was the credential, not the
   thresholds. (A future enhancement could raise the default threshold, but
   that's the operator's config.)

3. **Fail-safe exhaustion.** On exhausting `max_attempts`, ensure `backend` is
   disconnected/reconciled and the state file reflects "not connected" so `status`
   is honest and no half-tunnel lingers (the data-plane/teardown path already
   reconciles on `disconnect()`/drop; verify and keep).

## Project Structure

```
src/cli/vpn.rs   # MODIFY:
                 #  - add `initial: bool` to native_connect_once
                 #  - add pure select_password() + #[cfg(test)] tests
                 #  - run_vpn_on_native: initial=true
                 #  - native_supervise reconnect: initial=false (fresh OTP)
                 #  - on exhaustion: reconcile + honest state
```

## Complexity Tracking

No constitution violations. The change is a small, well-scoped correctness fix
with an isolated, unit-tested decision function.
