# ADR 0001 — Hand-rolled minimal netlink for rootless TUN/route setup

* Status: Accepted
* Deciders: akon maintainers
* Date: 2026-06-21
* Related: spec 006 (Native F5 VPN Backend)

## Context

The native F5 backend (spec 006) is a full in-process replacement for the
`openconnect` delegation. A core feature-parity requirement is **rootless
operation**: akon must run as the unprivileged user (so the OS keyring stays
accessible) with only the network setup requiring `CAP_NET_ADMIN`. The intended
deployment model is a **file capability** on the binary
(`setcap cap_net_admin+ep akon`).

The blocker: `LinuxTun::configure`/`Drop` and the teardown reconciler currently
shell out to `ip` and `sysctl`. A file capability is **not inherited by child
processes**, so a spawned `ip` runs without `CAP_NET_ADMIN` and fails when akon
is launched rootless via the file capability. To be genuinely rootless, the
link/address/MTU/route operations must be performed **in-process** so they run
under akon's own (file-capability-granted) credentials.

The networking operations we need are small and fixed:
- bring the link up, set MTU (`RTM_NEWLINK`/`RTM_SETLINK`),
- add/remove an address (`RTM_NEWADDR`/`RTM_DELADDR`),
- add/replace/remove routes incl. device-bound and via-gateway
  (`RTM_NEWROUTE`/`RTM_DELROUTE`),
- delete the interface (`RTM_DELLINK`).

`rp_filter` is set via `/proc/sys/net/...`, which is a plain file write (no child
process needed) and is unaffected by the capability-inheritance problem. DNS
configuration via `resolvectl`/`systemd-resolved` goes over D-Bus/polkit, does
**not** require `CAP_NET_ADMIN`, and therefore the `resolvectl` child works fine
rootless — DNS shell-outs are **not** part of this change.

Alternatives considered:
- **`rtnetlink` crate (pinned, + `tokio_socket`)**: ergonomic high-level async
  builders, battle-tested, but pulls the `netlink-*` dependency tree and must be
  pinned to remain MSRV-1.70 compatible. This is in tension with the project's
  established "no heavyweight required dependencies" stance (the HTTP/1.1 client
  and the F5 options XML parser are both hand-rolled for the same reason).
- **A privileged helper (setuid/setcap one-shot, or keep an elevated step)**:
  avoids netlink but does not achieve true in-process rootless operation; it
  reintroduces a privileged child and more moving parts.

## Decision

Implement a **small, hand-rolled netlink module** under
`akon-core/src/vpn/f5/netlink.rs` using the crate's existing `libc` (and `nix`)
dependencies — **no new crates**. It opens an `AF_NETLINK`/`NETLINK_ROUTE`
socket and sends the handful of `RTM_*` messages listed above, with ACK
(`NLM_F_ACK`) handling and error decoding. Message construction (headers,
attributes/`rtattr`, alignment) is **pure and unit-tested** with byte-level
assertions; only the socket send/recv is the thin effectful adapter.

`LinuxTun` uses this module instead of shelling out to `ip`. `rp_filter` is set
by writing `/proc/sys/...` directly. DNS continues to use the existing
`DnsApplier` (`resolvectl`/`resolvconf`) unchanged.

This matches the project's existing pattern (hand-rolled HTTP/XML), keeps the
dependency surface flat, is MSRV-1.70-safe, and makes the privileged operations
run in-process so a `cap_net_admin+ep` file capability is sufficient — no `sudo`,
no cap-dropping child processes.

## Consequences

- **Rootless parity becomes achievable**: with `setcap cap_net_admin+ep akon`,
  akon can configure the TUN, addresses, and routes as the user, with the keyring
  intact and no `sudo`. (File capabilities still do not elevate inside a user
  namespace, so rootless-container dev environments continue to need `sudo`; bare
  -metal Fedora/Ubuntu hosts get true rootless.)
- **No new dependencies / MSRV risk**: we own the netlink code; it builds on the
  existing `libc`/`nix`.
- **More code to maintain**: we hand-roll `rtattr` encoding and `RTM_*` request
  building. Mitigated by keeping message construction pure and unit-testing it
  byte-for-byte, and by the small, fixed set of operations.
- **Seam-isolated and testable offline**: pure message-builders are tested
  without privileges; the real socket round-trip is exercised in a throwaway
  network namespace (consistent with the methodology used for the data plane).
- **Diagnostics**: the previous `ip route show`/`ip addr show` debug dumps are
  no longer free; they are dropped or replaced with netlink-derived equivalents
  only where they add diagnostic value.
- If our netlink needs grow substantially later (e.g. policy routing, rules,
  DTLS-driven changes), revisiting `rtnetlink` is reasonable and would supersede
  this ADR.
