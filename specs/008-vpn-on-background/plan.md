# Implementation Plan: `akon vpn on` background mode and production log levels

**Branch**: `008-vpn-on-background` | **Date**: 2026-06-22 | **Spec**: [spec.md](./spec.md)

## Summary

Two independent, small changes:

**A — Background mode (FR-001..FR-005)**: After `vpn on` reaches `Connected`, the
terminal is returned to the user. The VPN supervisor continues as a detached
background process. Implementation: **pre-tokio double-fork** (safe because it
happens before any async runtime), with a **pipe** to relay the connect
result (success/failure + IP) to the foreground parent, which prints the result
and exits. The background child calls `setsid()`, redirects its stdio to an
akon log file, then starts the tokio runtime and runs the full native VPN as
before.

**B — Production log levels (FR-006..FR-009)**: All `[tun-cfg]` and `[tun-io]`
internal traces in `tun.rs`, and `[dns]` traces in `dns.rs`/`backend.rs`, are
moved behind the existing `AKON_F5_DEBUG=1` gate. Errors and warnings remain
unconditional.

## Technical Context

**Language/Version**: Rust 2021, MSRV 1.70 / CI toolchain 1.96
**Dependencies**: `nix` (already in workspace — `fork`, `setsid`, pipe I/O);
`libc` (already in akon-core — `open`, `dup2`); no new dependencies.
**Storage**: VPN state file (written by child before signalling parent, so `vpn
off`/`status` see it immediately). Log file: `~/.local/share/akon/vpn.log` (or
`$XDG_DATA_HOME/akon/vpn.log`), created by the child.
**Testing**: offline unit tests for the log-level gate (no AKON_F5_DEBUG →
no trace lines); the background flow is tested via integration: spawn `akon vpn
on` as a subprocess, assert it exits with 0 + confirmation output within a
bounded time.
**Platform**: Linux (`fork`/`setsid` are Linux/POSIX). Non-Linux: skip backgrounding,
run blocking as today.
**Constraints**: `fork()` MUST happen before `#[tokio::main]` starts the runtime
(forking inside a multi-threaded tokio runtime is unsafe). The pipe carries the
connect result from child to parent. No new binary entry points.

## Constitution Check

- [x] **Security-First**: No credentials touched. The pipe carries only the
  connection result (IP string, error string). The child inherits no secrets
  beyond what the process already has in memory (same credentials flow).
- [x] **Modular Architecture**: backgrounding logic is isolated in a
  `background::fork_and_connect()` helper; the VPN connect path in
  `run_vpn_on_native` is unchanged. Log-level gating is a pure mechanical change
  in `tun.rs` / `dns.rs` / `backend.rs`.
- [x] **Test-Driven Development**: Tests written first for (a) log-level gating
  (assert no trace lines without AKON_F5_DEBUG) and (b) the fork/pipe result
  relay logic (pure: encode/decode ConnectResult over the pipe).
- [x] **Observability**: The background child logs to `~/.local/share/akon/vpn.log`
  (the same information that previously went to the terminal). The foreground
  parent prints the connection summary. `journalctl` can still capture tracing
  output via the existing tracing setup.
- [x] **CLI-First Interface**: `vpn on` is unchanged UX-wise except it returns
  the prompt. `--foreground` flag explicitly preserves the blocking mode.
- [x] **Test Actors & Seam-Isolated Testing**: The fork/pipe result relay is pure
  (encode/decode `ConnectResult` as bytes over a pipe — tested offline). Log-
  level gating tested by asserting stderr content with/without the flag. The
  full background flow is integration-tested by spawning a real subprocess
  (bounded, no hang, no root needed for the test binary invocation check).

**Security-Critical Changes**: none (no auth/OTP/keyring/password/secret-config
paths touched).

## Design Decisions

1. **Pre-tokio fork (safe).** `fork()` is called in a single-threaded context
   before `#[tokio::main]`. The child side starts the tokio runtime and runs the
   VPN. The parent side waits for the pipe result, prints it, then exits.
   Using `nix::unistd::fork()` (already a dependency).

2. **Pipe as the connect result channel.**
   `pipe()` → `fork()`. Child writes a serialised `ConnectResult` (success +
   IP + device, or failure + message) to the write end after `Connected`/`Failed`
   is received. Parent reads from the read end (bounded: 10s timeout before
   treating as failure). Parent exit code mirrors the verdict.

3. **Child detaches.** After `fork()`, child calls `setsid()` (new session, no
   controlling terminal), then redirects `stdin → /dev/null`,
   `stdout/stderr → ~/.local/share/akon/vpn.log` (appended, created if needed).
   Then starts the tokio runtime and runs `run_vpn_on_native` as today.

4. **`--foreground` flag.** When passed, skip the fork entirely — run blocking as
   before. Useful for CI / supervised environments / debugging.

5. **Log-level gate is purely mechanical.** Every `eprintln!("[tun-cfg]`... and
   `eprintln!("[tun-io]`... in `tun.rs`, and `eprintln!("[dns]`... in
   `dns.rs`/`backend.rs` that are currently unconditional are wrapped in
   `if debug_enabled() { … }` — using the existing `http::debug_enabled()` helper
   that checks `AKON_F5_DEBUG`. Error/WARN lines remain unconditional.

## Project Structure

```
src/cli/
├── vpn.rs       # MODIFY: vpn_on entry: fork_and_connect() when !--foreground
└── background.rs  # NEW: fork_and_connect(), ConnectResult pipe encode/decode

akon-core/src/vpn/f5/
├── tun.rs       # MODIFY: gate [tun-cfg]/[tun-io] behind debug_enabled()
├── dns.rs       # MODIFY: gate [dns] traces behind debug_enabled()
└── backend.rs   # MODIFY: gate [dns] + [tun-cfg] ERROR stays unconditional
```

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|--------------------------------------|
| `fork()` in a CLI binary | Only safe way to background before tokio starts; the TUN is opened inside the async runtime so post-tokio fork is not safe | `nohup`/`setsid` from a shell wrapper — doesn't give clean exit-code relay back to the user; `re-exec` — can't re-attach to the already-open TUN fd |
