# ADR 0002 — Remove openconnect; the native F5 backend is the only VPN backend

* Status: Accepted
* Deciders: akon maintainers
* Date: 2026-06-21
* Related: ADR 0001 (hand-rolled netlink), spec 006 (Native F5 VPN Backend)
* Supersedes: the openconnect-delegation design from spec 002
  (`002-refactor-openconnect-to`) and FR-013 of spec 006 ("openconnect remains
  the default")

## Context

akon historically delegated all VPN work to the external `openconnect` binary,
spawned via `sudo` (spec 002). Spec 005 introduced a backend-agnostic
`VpnBackend` boundary and a test-actors framework; spec 006 then implemented a
pure-Rust `NativeF5Backend` as an opt-in (`native_backend = true`) replacement.

The native backend is now **production-proven** end-to-end:
- control plane + PPP validated against the real appliance,
- data plane carrying real bidirectional traffic to internal hosts (a 3-minute
  interactive hold-open session over production),
- **rootless** operation via in-process netlink (ADR 0001) under a
  `cap_net_admin+ep` file capability — no `sudo`, no child `ip`/`openconnect`,
- complete, idempotent host teardown (`HostTeardownPlan`/`teardown_host`) that
  restores routing/DNS/rp_filter even after a SIGKILL.

Keeping both backends imposes ongoing cost: a second event vocabulary
(`ConnectionEvent` + `OutputParser` regexes) bridged by an adapter, a separate
process/daemon lifecycle (PID discovery via `pgrep`, SIGTERM/SIGKILL, orphan
reaping, a spawned reconnection daemon), an external runtime dependency
(`openconnect`, `procps`) and a passwordless-sudo install step, and duplicated
CLI branches in `vpn on/off/status`. The native path removes all of this.

## Decision

**Remove the openconnect backend entirely and make `NativeF5Backend` the only
VPN backend.** This is a breaking change.

Concretely:
- Delete the openconnect implementation: `openconnect_backend.rs`,
  `cli_connector.rs`, `output_parser.rs`, the openconnect `process.rs`, the
  duplicate `connection_event.rs` (`ConnectionEvent`/openconnect `DisconnectReason`),
  and `src/daemon/` (the openconnect orphan-reaper). Keep
  `system_effects::TermSignal` (used by the test `SimulatedBackend`).
- Remove the `native_backend` config flag; the native path is unconditional for
  the F5 protocol.
- Collapse the CLI: `vpn on` always uses the native backend and supervises in
  process; `vpn off` always replays the persisted `HostTeardownPlan`; `vpn
  status` reads the backend-agnostic state file. Remove the
  `which::which("openconnect")` check, the spawned reconnection daemon, and the
  PID-kill teardown.
- Remove openconnect-only error variants and their exit-code mappings; drop the
  `which` (and dead `bindgen`) dependencies and `regex` from akon-core; drop
  `openconnect`/`procps` from deb/rpm metadata and the passwordless-sudo install
  steps.
- Delete openconnect-specific tests; keep the backend-agnostic and native suites.
- Update all instructions (README, packaging, Makefile, CI, specs) to the native,
  rootless model: install via `setcap cap_net_admin+ep`, run as the user, no sudo,
  no openconnect.

The runtime model becomes: akon runs as the user (keyring intact); the only
privilege is `CAP_NET_ADMIN` for TUN + netlink, granted by a file capability on
the binary.

## Consequences

- **Breaking change** for operators: openconnect is no longer used or required;
  `native_backend` is gone (a stale `native_backend = true/false` in config is
  ignored/removed). Installation changes from "install openconnect + passwordless
  sudo" to "`setcap cap_net_admin+ep /usr/bin/akon`". Documented in the changelog
  and README.
- **Simpler, dependency-light binary**: no external `openconnect`/`procps`, no
  process-spawn/PID-kill/daemon machinery, a single event vocabulary
  (`LifecycleEvent`), one CLI path. Smaller attack surface and less to maintain.
- **Rootless by default**: no `sudo` for normal operation on bare-metal
  Fedora/Ubuntu (file capabilities still don't elevate inside user namespaces, so
  rootless-container dev envs still need `sudo` or `--cap-add`).
- **Protocol scope narrows to F5** (what akon actually targets). Non-F5
  openconnect protocols are no longer supported; reintroducing another protocol
  would mean a new native backend behind the same `VpnBackend`/`Transport` seams.
- **DTLS/UDP** remains unimplemented (TLS-only); acceptable since the appliance
  works over TLS and `no_dtls = true` is satisfied.
- History preserved: the openconnect specs (001, 002, 004) remain as archived
  design records; this ADR supersedes their operative decisions.
