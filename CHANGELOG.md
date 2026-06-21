# Changelog

All notable changes to akon are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

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
