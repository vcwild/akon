# Changelog

All notable changes to akon are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

## [2.1.1] — 2026-06-22

### Fixed

- **Packaging: declare the `libcap` dependency.** The post-install scripts run
  `setcap cap_net_admin+ep` to grant the `CAP_NET_ADMIN` akon needs to create the
  TUN device. `setcap` was not declared as a package dependency, so on a host
  without it the install only printed a warning and akon silently failed to
  connect until libcap was installed manually. The `.deb` now depends on
  `libcap2-bin` and the `.rpm` requires `libcap`, so the package manager pulls it
  in automatically.

## [2.1.0] — 2026-06-22

Reliability and UX hardening for the native F5 client. No breaking changes;
upgrade in place.

### Fixed

- **Stable long-lived tunnels (no more periodic drops).** The F5 server runs
  dead-peer-detection by sending LCP Echo-Requests (~every 30 s) and tearing the
  tunnel down after a few go unanswered (~149 s). The data plane previously
  ignored inbound LCP control frames, so the server declared the client dead and
  dropped the connection on a fixed interval. akon now replies to the server's
  Echo-Requests with an Echo-Reply, keeping the tunnel up indefinitely.
- **Robust reconnection.** Supervision is now event-driven: a dropped tunnel is
  detected immediately (rather than by polling) and reconnects, regenerating a
  fresh OTP per attempt so time-based one-time passwords are not reused.
- **No password prompts for DNS.** A scoped polkit rule authorizes the
  `systemd-resolved` DNS operations akon needs, so `vpn on`/`off` no longer
  prompt for a password mid-connection.
- **`vpn off` no longer prints spurious errors.** DNS revert no longer leaks
  `Failed to resolve interface "tunN": No such device` when the link is already
  gone (resolved auto-reverts in that case).

### Added

- **Background mode** for `akon vpn on` (returns your terminal once connected);
  use `--foreground` to keep it attached.
- **`vpn status`** reflects the real tunnel interface state rather than a process
  handle.
- **Timestamped debug logs** under `AKON_F5_DEBUG=1`, with periodic throughput
  summaries instead of per-packet dumps, for readable soak logs.

## [2.0.0] — 2026-06-21

### ⚠️ Breaking changes

akon is now a **native, in-process F5 BIG-IP SSL VPN client** written in pure
Rust. The `openconnect` delegation has been **removed entirely**. See
`docs/adr/0002-remove-openconnect-native-f5-is-the-only-backend.md`.

- **`openconnect` is no longer used or required.** akon no longer spawns
  `openconnect` (or any child process) for the VPN protocol, and the
  `openconnect`/`procps` package dependencies are gone.
- **Runtime model changed: no `sudo`.** akon runs as your user (so the keyring
  stays accessible). The only privilege needed is `CAP_NET_ADMIN` for the TUN
  device and route setup, granted once as a **file capability**:

  ```bash
  sudo setcap cap_net_admin+ep "$(command -v akon)"
  ```

  Packaging post-install scripts and `make install` now do this automatically
  (and remove the legacy `/etc/sudoers.d/akon` passwordless-sudo file). Requires
  `libcap` (`setcap`): `apt install libcap2-bin` / `dnf install libcap`.
- **Config: the `native_backend` flag is removed.** The native backend is always
  used for `protocol = "f5"`. A leftover `native_backend = …` line is harmlessly
  ignored.
- **Protocol scope is F5.** Other openconnect protocol identifiers remain
  parseable in config for forward-compatibility but are not implemented by the
  native client.

### Added

- Native F5 client: F5 framing (encap + HDLC/FCS16), PPP (LCP/IPCP/IP6CP)
  negotiation, HTTP auth + XML config, TLS transport, and orchestration behind a
  backend-agnostic `VpnBackend` boundary — all validated test-first against an
  in-memory test-actors framework and byte-exact wire vectors.
- **In-process netlink** configuration of the TUN device, addresses, MTU, and
  routes (no `ip`/`sysctl` child processes), enabling true rootless operation.
- **Guaranteed host restore:** `akon vpn off` replays a persisted host-teardown
  plan (tun, server-pin route, `rp_filter`, DNS) — idempotent, and works even if
  the `vpn on` process was killed.
- In-process health-checked reconnection (honors the `[reconnection]` config).
- Containerized rootless validation and gated production sign-off tests.

### Removed

- `openconnect_backend`, `cli_connector`, `output_parser`, the openconnect
  `process` module, `connection_event`, `system_effects`, and the spawned
  reconnection daemon.
- Dependencies: `which`, `bindgen`, `daemonize` (and `regex` from akon-core).
- openconnect-specific error variants (`OpenConnectError`, `ProcessSpawnError`,
  `TerminationError`, `ParseError`).

### Migration

1. Update akon (or `make install`).
2. Ensure the capability is set: `getcap "$(command -v akon)"` should show
   `cap_net_admin=ep`; if not, run the `setcap` command above.
3. Remove any `native_backend = …` line from `~/.config/akon/config.toml`
   (optional — it is ignored).
4. Run akon **without** `sudo`: `akon vpn on`. You may uninstall `openconnect`.
