# Feature Specification: `akon vpn on` background mode and production log levels

**Feature Branch**: `008-vpn-on-background`
**Created**: 2026-06-22
**Status**: Draft
**Input**: User description: "once the vpn connection is established we want to
give the terminal back to the user (send to background). also in the production
environment we don't want to expose the tun-cfg debug settings, only the basic
stuff."

## Overview

Two related UX concerns:

1. **Background mode**: `akon vpn on` currently blocks the terminal for the entire
   lifetime of the VPN session. Once connected, it should return the shell prompt
   to the user while the VPN runs in the background — compatible with the native
   in-process model (the process can't `fork` after opening the TUN, but it can
   redirect stdio and signal the parent via a pipe pair).

2. **Production log levels**: The native backend emits detailed internal state
   (`[tun-cfg]`, `[tun-io]`) unconditionally to stderr. These are diagnostic
   traces for development, not information a production user needs to see. Only
   the final "connected" line and errors should appear by default; the rest must
   be gated behind the existing `AKON_F5_DEBUG=1` flag (which already gates
   `[f5-data]`, `[f5-ppp]`, `[f5-http]`).

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Terminal returned after connect (Priority: P1)

As a user running `akon vpn on`, once the VPN is established I get my shell
prompt back and can continue working in the same terminal. The VPN continues
running in the background. I can stop it later with `akon vpn off`.

**Why this priority**: Blocking the terminal is the most disruptive aspect of the
current UX. Users expect `akon vpn on` to behave like a service start command —
it starts the service, confirms success, and returns. Having to open a second
terminal for a background flag is friction.

**Independent Test**: `akon vpn on` in a simulated environment exits (returns
the parent shell) within a bounded time after the `Connected` lifecycle event is
emitted. The VPN supervisor process is still running after the parent exits.

**Acceptance Scenarios**:

1. **Given** valid credentials and a reachable VPN server, **When** I run
   `akon vpn on`, **Then** the command prints a connected confirmation and
   returns to the shell prompt once the connection is established.
2. **Given** `akon vpn on` has returned the prompt, **When** I run
   `akon vpn status`, **Then** it reports connected (exit 0) with the correct
   device and IP.
3. **Given** the VPN is running in background, **When** I run `akon vpn off`,
   **Then** the background supervisor is signalled and the tunnel tears down
   cleanly with host state restored.
4. **Given** the connection attempt fails (bad credentials / unreachable server),
   **When** I run `akon vpn on`, **Then** the error is printed to the terminal
   and the command exits with a non-zero code (no orphaned background process).

---

### User Story 2 — Clean production output by default (Priority: P1)

As a user running `akon vpn on` in a production host, I only see the essential
connection status — connecting, authenticated, connected — and any errors. I do
not see internal implementation traces (`[tun-cfg] full_tunnel=true`,
`[tun-cfg] default-half …`, `[tun-io] read N bytes`, etc.) unless I explicitly
opt in to debug output.

**Why this priority**: The current output leaks internal routing decisions,
interface names, IP assignments, and packet-level traces to the terminal and any
system logs that capture stderr. This is noise in production and may inadvertently
expose topology details.

**Independent Test**: Running `akon vpn on` (in a test harness) with no
`AKON_F5_DEBUG` env variable produces only user-facing lines; all `[tun-cfg]`,
`[tun-io]`, `[dns]` internal traces are absent from stderr.

**Acceptance Scenarios**:

1. **Given** `AKON_F5_DEBUG` is not set, **When** I run `akon vpn on`, **Then**
   stderr contains no `[tun-cfg]`, `[tun-io]`, or `[dns]` trace lines.
2. **Given** `AKON_F5_DEBUG=1` is set, **When** I run `akon vpn on`, **Then**
   all existing trace lines are still present (no regression for dev/debug).
3. **Given** a configure error (e.g. interface setup fails), **When** it occurs,
   **Then** the error IS shown to the user regardless of the debug flag (errors
   are always visible).

---

### Edge Cases

- Connection fails before backgrounding → process stays in the foreground until
  the error is reported; exits non-zero; no orphaned background process.
- `akon vpn on --foreground` (or no backgrounding if the session is not a tty)
  → keep the existing blocking behaviour as an explicit opt-out for scripts /
  CI / supervised environments that expect `vpn on` to block.
- Reconnection (health-check failure while backgrounded) → the background
  supervisor reconnects in-process; no terminal interaction needed.
- `akon vpn off` against a backgrounded session → the existing state-file +
  interface teardown path works unchanged (no dependency on the terminal).

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `akon vpn on` MUST return the terminal to the user after the
  connection reaches `Connected`, with the VPN supervisor continuing in the
  background.
- **FR-002**: The background supervisor MUST be the same in-process native F5
  backend (not a `fork`/`exec` of a separate binary); stdio is redirected after
  connect rather than forking before.
- **FR-003**: If the connection attempt fails before `Connected`, `akon vpn on`
  MUST stay in the foreground, print the error, and exit non-zero with no
  orphaned background process.
- **FR-004**: `akon vpn on --foreground` (or equivalent flag) MUST keep the
  existing blocking behaviour for scripted / supervised use.
- **FR-005**: After backgrounding, `akon vpn status` and `akon vpn off` MUST
  work correctly (the state file is written before backgrounding; the interface
  is the ground truth for status).
- **FR-006**: All `[tun-cfg]` and `[tun-io]` internal trace lines MUST be gated
  behind `AKON_F5_DEBUG=1`. They MUST NOT appear on stderr by default.
- **FR-007**: `[dns]` internal trace lines MUST also be gated behind
  `AKON_F5_DEBUG=1`. DNS errors and warnings MUST remain visible regardless.
- **FR-008**: Errors and warnings from `configure()` (interface setup failures,
  route failures) MUST remain visible to the user regardless of `AKON_F5_DEBUG`.
- **FR-009**: The `AKON_F5_DEBUG=1` flag MUST continue to enable ALL existing
  trace output with no regression.
- **FR-010**: The background process MUST not print to the original terminal after
  the parent has returned (stdout/stderr redirected to a log file or `/dev/null`
  after backgrounding).

### Key Entities

- **VPN supervisor process**: the akon process that runs the in-process native F5
  backend after connecting. In background mode it detaches from the terminal
  after confirming `Connected`.
- **Log level**: `production` (default — errors + user-facing lines only) vs.
  `debug` (`AKON_F5_DEBUG=1` — all traces).

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `akon vpn on` returns the shell prompt within ~5 seconds of the VPN
  being established (bounded by the connection handshake time, not the session
  lifetime).
- **SC-002**: A default `akon vpn on` run produces zero `[tun-cfg]`, `[tun-io]`,
  or `[dns]` lines on stderr.
- **SC-003**: `AKON_F5_DEBUG=1` restores all trace output (no regression).
- **SC-004**: After `akon vpn on` returns, `akon vpn status` reports connected
  and `akon vpn off` disconnects cleanly with full host restore.
- **SC-005**: Connection failures are reported to the terminal with a non-zero
  exit and no orphaned process.
- **SC-006**: All behaviours are covered by tests compatible with the standard
  `cargo test` CI run (offline, no root, deterministic, no hang).
