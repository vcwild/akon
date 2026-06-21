# Feature Specification: Native F5 VPN Backend (openconnect replacement)

**Feature Branch**: `006-native-f5-backend`
**Created**: 2026-06-21
**Status**: COMPLETE — native F5 is the only backend (openconnect removed in v2.0.0; see ADR 0002). Control plane + data plane PRODUCTION-PROVEN; rootless runtime via in-process netlink (validated in-container as non-root). FR-003 proactive keepalive marked Won't Do (app-layer health check covers liveness).
**Input**: User description: "Use the test actors framework as ground truth and implement a full replacement of the openconnect backend for the F5 VPN protocol. Clone/inspect openconnect for protocol details. If the framework lacks features to test the replacement, extend the framework first, then circle back. Loop until complete."

## Implementation Status (updated 2026-06-21)

The native F5 backend has progressed from a **control-plane handshake only**
to a **functional in-process VPN with a production-proven data plane** (real user
traffic routed through a real TUN against the live appliance). An honest
DONE-vs-remaining summary:

**DONE**
- **Control plane**: HTTP auth (`MRHSession`+`F5_ST` cookies) → profile/options
  XML config → tunnel upgrade (`/myvpn`) → PPP (LCP/IPCP) to "network up", all
  over the `Transport` seam.
- **Data plane packet pump**: a bidirectional pump (`run_data_plane`) moving IP
  packets TUN ↔ F5 framing ↔ transport — OS-originated packets are wrapped in
  PPP (`wrap_ip_in_ppp`) and F5-encapsulated; inbound frames are decapsulated,
  filtered to IP payloads (`ppp_payload_if_ip`), and written to the TUN device.
- **Real Linux TUN device**: `LinuxTun` opens `/dev/net/tun` via
  `ioctl(TUNSETIFF)` and applies the negotiated `TunConfig` (address, MTU,
  routes) using `ip` tooling, behind the existing `TunDevice` seam.
- **Graceful teardown (FR-009)**: `graceful_teardown` sends a PPP
  Terminate-Request, then the F5 `vdesk/hangup.php3?hangup_error=1` logout, then
  closes the transport — best-effort, idempotent, bounded.
- **Production constructor**: `connect_from_config` builds a real TLS transport
  to the configured server (default port 443, via `split_host_port`) and a real
  `LinuxTun` directly from a `VpnConfig` — the constructor the CLI uses.
- **CLI wiring**: `akon vpn on` routes to the native backend when
  `native_backend = true` (new `VpnConfig` field) and `protocol = f5`, feeding
  the keyring-generated PIN+OTP password; the native client runs in-process
  (`run_vpn_on_native`).
- **Real TLS end-to-end test**: validated against a real local TLS server (in
  addition to the offline fake F5 server), exercising the production transport
  path.
- **Real F5 HTML auth-form parsing + multi-step OTP loop**: `auth.rs`
  (`F5AuthForm::parse` / `build_submission`) parses the `auth_form`, preserves
  hidden fields, and fills username + password where the password is akon's
  pre-composed **PIN+OTP** (since `generate_password` returns PIN+OTP
  concatenated). `backend.rs::authenticate` GETs the login page, parses the
  form, POSTs to its action, follows redirects, and loops until `MRHSession` +
  `F5_ST` appear — supporting multi-step OTP-form logins.
- **Host DNS application (Fedora/Ubuntu)**: `dns.rs` provides a `DnsApplier`
  seam and `SystemDnsApplier` that detects `systemd-resolved` (the default on
  Fedora and Ubuntu) and applies the negotiated DNS servers/search domains via
  `resolvectl dns`/`resolvectl domain`, with `resolvconf` and `/etc/resolv.conf`
  fallbacks. DNS is applied after the TUN is configured and reverted on
  teardown.
- **In-process reconnection + `lazy_mode`**: `run_vpn_on_native` generates a
  fresh PIN+OTP per attempt, persists state, and runs `native_supervise` — an
  in-process health-check (`HealthChecker` against `health_check_endpoint`) plus
  exponential-backoff reconnection loop honoring the `[reconnection]` policy.
  `lazy_mode` and `no_dtls = true` (the native path is TLS-only) are satisfied.

**DONE — containerized real-host validation (Podman)**
- A containerized integration test (`akon-core/tests/native_f5_podman_tests.rs`)
  runs a **real TLS F5 server** (the `f5_test_server` binary, driving the genuine
  `F5ServerActor`) inside a Podman container, and drives the native backend
  against it over a **real published TCP port + TLS handshake** — full network
  isolation, **no side effects on the host**.
- The native client is also run **inside real Fedora and Ubuntu containers**
  (`f5_test_client` binary) on a shared Podman network, validating both the
  TLS connect-to-`Connected` flow and the **distro-specific DNS application**
  (`SystemDnsApplier` → `resolvectl`/`resolvconf`/`resolv.conf`) on each distro.
  Both report `RESULT: ok`. The tests are **opt-in** (`AKON_RUN_PODMAN_TESTS=1`),
  self-skip when Podman is unavailable, are bounded, and always tear their
  containers/network down.
- Run with:
  `AKON_RUN_PODMAN_TESTS=1 cargo test -p akon-core --features test-actors --test native_f5_podman_tests -- --test-threads=1`

**DONE — production sign-off test (gated, generic, operator-run)**
- `tests/production_signoff_test.rs` is the final acceptance gate: it connects
  the native backend to the operator's **own** real F5 appliance, reaches
  `Connected`, and disconnects immediately. It reads the server, username, and
  PIN+OTP credentials entirely from the operator's local `~/.config/akon/config.toml`
  and keyring at run time — **no production endpoint, username, or network is
  hardcoded anywhere in akon**.
- It is **control-plane-only** (`connect_control_plane_only`): it creates no TUN
  device and changes no routes/DNS, so it validates reachability + protocol
  correctness against the live server **without disrupting the operator's own
  connectivity**. It is bounded (cannot hang).
- It is **disabled by default** and requires an explicit double opt-in so it can
  never run accidentally, in CI, or in the normal suite:
  ```text
  AKON_SIGNOFF_PRODUCTION=1 \
  AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION \
  cargo test --test production_signoff_test -- --nocapture
  ```
- It is intended to be run **once, by a human, at final sign-off**, after all
  prior layers (pure protocol, in-memory actors, real-local-TLS, Podman
  Fedora/Ubuntu) are green.

> ✅ **PRODUCTION SIGN-OFF ACHIEVED.** The native F5 backend was validated
> against a **real production F5 appliance**: it authenticated with real PIN+OTP
> keyring credentials over real TLS, completed the full handshake
> (auth → config → `/myvpn` → PPP LCP/IPCP to network-up), was assigned a real
> tunnel IP, and disconnected cleanly. The control plane and PPP negotiation are
> therefore confirmed end-to-end against production, not just emulation.
>
> Divergences discovered during sign-off (each reproduced offline before fixing):
> real F5 closes the connection between requests (`Connection: close`) → reconnect
> per request; AnyConnect-compatible `User-Agent` + `X-Pad` required; tolerant PPP
> option parsing (unknown options kept, overruns stop the loop); IPCP must echo
> NAKed DNS values (RFC1877) to avoid an infinite NAK loop; IP6CP must be
> Configure-Rejected (the server retransmits until answered). All are covered by
> the byte-accurate regression test `converges_against_real_appliance_ipcp_nak_sequence`
> and the realistic closing-server end-to-end test.

**DONE — production DATA-PLANE sign-off (the "it's a real VPN" gate)**
- The native backend was validated **carrying real user traffic through a real
  TUN against the live production appliance** (not just the control plane). The
  gated `tests/production_dataplane_signoff_test.rs` connected, reached
  network-up, **resolved a VPN-only name (`intranet.example.com`) by querying the VPN
  DNS through the tunnel**, routed that target's `/32` via the tun, and **opened
  a TCP connection to it through the tunnel** — proving bidirectional data-plane
  forwarding end-to-end. It then tore everything down with **no leaked interface,
  routes, server-pin, or rp_filter**, and the host's default route + DNS fully
  recovered.
- **MTU is derived from the negotiated MRU** (`0x0583 = 1411`), no longer a fixed
  1400 (`negotiator.negotiated_mtu()`).
- Two data-plane bugs were found and fixed during this work, each reproduced
  **offline** (in a throwaway network namespace) before any production run:
  (1) `LinuxTun` used `tokio::fs::File` for the TUN fd — buffered/offset I/O made
  packets just written read straight back (an echo/loop); fixed with `AsyncFd` +
  raw `read(2)`/`write(2)` syscalls (one packet per syscall). (2) `Connected` was
  emitted before `configure()` ran and `configure()` errors were swallowed; now
  `LinkUp`/`Connected` are emitted only after the interface is configured and a
  configure failure surfaces as `Failed`. Both are locked by the gated
  `native_f5_netns_roundtrip_tests.rs` regression (asserts `RESULT: ok` round-trip
  AND `TEARDOWN: ok`).
- **Symmetric host teardown**: `akon vpn off` is now native-aware. Connect-time
  host mutations (tun device, non-device-bound server-pin route, original
  `rp_filter` values, DNS interface) are recorded in a persistable
  `HostTeardownPlan` written to the state file, and `vpn off` replays an
  idempotent `teardown_host` reconciler — so a production host is restored even
  if the `vpn on` process was SIGKILL'd and never ran its own cleanup.

**DONE — rootless runtime (openconnect feature parity)**
- All host network configuration (link up, MTU, address, routes, route dump,
  interface delete) is now performed **in-process via netlink** (`f5/netlink.rs`,
  a hand-rolled minimal `NETLINK_ROUTE` client — see ADR 0001), not by shelling
  out to `ip`. `rp_filter` is set by writing `/proc/sys` directly. The only
  remaining external command is `resolvectl` for DNS, which talks to
  systemd-resolved over D-Bus/polkit and does **not** require `CAP_NET_ADMIN`, so
  it works rootless.
- Because nothing is spawned for the privileged operations, a **`cap_net_admin+ep`
  file capability** on the akon binary is now sufficient: akon runs **as the
  user** (keyring intact) with **no `sudo`** and no cap-dropping child process.
- **Validated, fully containerized**: `native_f5_podman_tests::
  rootless_dataplane_runs_in_container_as_user` builds an image that grants the
  probe `cap_net_admin+ep` and runs it as a **non-root user**, with
  `--cap-add NET_ADMIN --device /dev/net/tun --network none`. The probe brings up
  a real TUN, configures address + full-tunnel routes via netlink, completes a
  data-plane round-trip, and tears down — all unprivileged, in complete container
  isolation (`./test-support/run-rootless-validation.sh`). Earlier manual
  validation also confirmed the file-capability path works for a normal user.
- **Test host-safety policy**: tests that create a real TUN / touch routing
  **refuse to run in the host network namespace** (the probe requires
  `AKON_PROBE_ISOLATED=1` and verifies no real uplink default; the real-TUN tests
  skip unless in an isolated netns). DNS revert is recorded in the teardown plan
  **only** when a host-mutating `DnsApplier` actually applied DNS
  (`DnsApplier::mutates_host()`), so test/container runs never issue `resolvectl`
  against the host resolver.

**DONE — openconnect removed; native is the only backend (v2.0.0)**
- The `native_backend` flag, the openconnect backend/connector/parser/process/
  daemon, and the external `openconnect`/`procps` dependencies are gone. `vpn
  on/off/status` use the native path unconditionally. Install grants
  `cap_net_admin+ep` (no sudo). See ADR 0002 and the CHANGELOG.

**REMAINING / NOT YET** (optional, non-blocking)
- **FR-003 proactive keepalive (LCP Discard-Request): WON'T DO** — the in-process
  HTTP health check covers liveness; the appliance does not require client
  keepalives (proven in production). Echo-Reply (responder) is implemented.
- **DTLS (UDP) transport**: TLS-only today; a UDP/DTLS path is not implemented
  (it would slot behind the same `Transport` seam, reintroducing PPP retransmit
  timers, without touching the validated layers). `no_dtls = true` is therefore
  already satisfied.
- **resolv.conf-file fallback restore-on-revert**: when the host has neither
  `systemd-resolved` nor `resolvconf`, the direct `/etc/resolv.conf` rewrite is
  best-effort and is not restored on revert.

### Enabling the native backend for the production use case

To use the native backend for a real F5 deployment, add `native_backend = true`
under `[vpn]` in `~/.config/akon/config.toml` (server/username are the
operator's own values, never hardcoded in akon):

```toml
[vpn]
server = "vpn.example.com"   # your F5 server
username = "your-username"   # your VPN username
protocol = "f5"
timeout = 30
no_dtls = true
lazy_mode = true
native_backend = true        # use akon's in-process native F5 client

[reconnection]
max_attempts = 10
base_interval_secs = 5
backoff_multiplier = 2
max_interval_secs = 30
consecutive_failures_threshold = 1
health_check_interval_secs = 60
health_check_endpoint = "https://www.example.org"
```

A typical enterprise F5 configuration is fully supported by the native backend:
the F5 protocol (PIN+OTP auth-form login), `no_dtls = true` (TLS-only by design),
`lazy_mode`, and the entire `[reconnection]` policy (driven by the in-process
`native_supervise` health-check + backoff loop). DNS for the tunnel is applied
on `systemd-resolved` hosts (Fedora/Ubuntu) automatically. The native backend
is opt-in via `native_backend = true`; without it, openconnect remains the
default.

## Problem Statement

akon currently delegates all VPN work to the `openconnect` binary (spawned via `sudo`). This is the dependency we want to remove. Spec 005 introduced the backend-agnostic [`VpnBackend`] boundary and a test actors framework precisely to make this removal safe. This feature delivers the replacement: a **native, in-process F5 BIG-IP SSL VPN client** implemented in pure Rust, validated by the actors framework as ground truth — no `openconnect`, no `sudo`-spawned child for the protocol itself.

F5 is a **PPP-over-HTTPS** protocol (confirmed from openconnect `f5.c`): HTTP(S) auth → XML config → an HTTP "tunnel upgrade" → a PPP session (LCP/IPCP) framed with an F5-specific 4-byte encapsulation. Each of these layers is independently implementable and independently testable.

## Strategy: framework-as-ground-truth, test-first

The native backend is built **layer by layer, test-first**, with the test actors framework as the oracle:
1. If a layer can't be tested with the current framework, the framework is extended first (a fake F5 server actor, an in-memory transport, a PPP peer actor), then the layer is implemented against it.
2. Every layer is pure/seam-isolated so it needs no real network or root to test.
3. The final `NativeF5Backend` implements the same [`VpnBackend`] trait as the simulated and openconnect backends, so the **existing cross-backend equivalence machinery proves it behaves identically** before it could ever become the default.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Native F5 wire framing (Priority: P1)

As an akon developer, I can encode/decode F5's PPP-over-HTTPS framing (the `0xf500|len` pre-PPP header, the PPP `FF 03 proto` header, and the HDLC variant) in pure Rust, validated by byte-exact test vectors, so the data path is correct without a real server.

**Why this priority**: Framing is the foundation of the tunnel and is fully deterministic — perfect to build test-first. Wrong framing means no packet ever flows.

**Independent Test**: Feed known PPP payloads through the encoder and assert exact wire bytes (`f5 00 <len> <ppp>`); feed known wire bytes through the decoder and assert the recovered PPP frames, including concatenated frames in one buffer and the HDLC escape/FCS path.

**Acceptance Scenarios**:
1. **Given** a PPP IP payload, **When** F5-encapsulated, **Then** the bytes are `0xF5 0x00` + big-endian length + payload.
2. **Given** a buffer with two concatenated F5 frames, **When** decoded, **Then** both frames are recovered in order.
3. **Given** an HDLC-framed LCP frame, **When** decoded, **Then** byte-unstuffing + FCS check succeed and the payload matches.

---

### User Story 2 - PPP/LCP/IPCP negotiation (Priority: P1)

As an akon developer, I can run the PPP control negotiation (LCP up, then IPCP to obtain the assigned IP + DNS) as a deterministic state machine driven by a simulated PPP peer, so the tunnel reaches the "network up" state offline.

**Why this priority**: Without LCP+IPCP completing, the tunnel never carries IP traffic. It's the core protocol logic and must be proven against a peer that ACKs/NAKs like a real F5 server.

**Independent Test**: Drive the PPP state machine with a fake peer that ACKs our LCP Config-Request and NAKs our IPCP IP/DNS requests with concrete values; assert the machine reaches `Network` state with the negotiated IPv4 address and DNS servers.

**Acceptance Scenarios**:
1. **Given** a peer that ACKs LCP, **When** negotiation runs, **Then** LCP reaches Opened.
2. **Given** a peer that NAKs IPCP with an IP and DNS, **When** negotiation runs, **Then** IPCP reaches Opened with that IP/DNS recorded.
3. **Given** a peer that sends an LCP Echo-Request, **When** received, **Then** an Echo-Reply is produced (DPD).

---

### User Story 3 - F5 HTTP auth + XML config (Priority: P2)

As an akon developer, I can perform the F5 HTTP auth handshake (form/cookie logic yielding `MRHSession`+`F5_ST`) and parse the profile/options XML (session id, `ur_Z`, ipv4/ipv6/hdlc flags, DNS, routes) against a fake F5 server, so the pre-tunnel phase works offline.

**Why this priority**: Needed to reach the tunnel, but it is conventional HTTP/XML work and lower-risk than framing/PPP. Builds on the transport seam from US1/US2.

**Independent Test**: Run the auth+config exchange against a fake F5 server actor that returns a login form, sets both cookies on credential POST, and serves profile/options XML; assert the extracted cookies, session id, `ur_Z`, and config (ipv4 on, DNS list).

**Acceptance Scenarios**:
1. **Given** a fake server serving an `auth_form`, **When** credentials are posted, **Then** both `MRHSession` and `F5_ST` are captured and auth is considered successful.
2. **Given** missing `F5_ST`, **When** auth runs, **Then** it is treated as not-yet-authenticated (failure if exhausted).
3. **Given** options XML with `Session_ID`, `ur_Z`, `IPV4_0`, and `DNS0`, **When** parsed, **Then** those values are extracted; missing `ur_Z`/`Session_ID` is an error.

---

### User Story 4 - End-to-end native backend equivalence (Priority: P1)

As an akon developer, I can run the **same** scenario suite (from spec 005) against `NativeF5Backend` and `SimulatedBackend` and observe equivalent lifecycle timelines, proving the native backend behaves identically to ground truth before it could become the default.

**Why this priority**: This is the deliverable's proof of correctness and the whole point of building the framework first. It demonstrates the openconnect replacement is safe.

**Independent Test**: Wire `NativeF5Backend` to a fake F5 server actor over an in-memory transport; run the connect → connected → disconnect scenario; assert the lifecycle subsequence matches the simulated backend's.

**Acceptance Scenarios**:
1. **Given** a fake F5 server scripted for a successful session, **When** `NativeF5Backend` connects, **Then** the lifecycle reaches `Connected` with the server-assigned IP.
2. **Given** the same scenario run against the simulated and native backends, **When** compared, **Then** the lifecycle timelines are equivalent.
3. **Given** a fake server that rejects credentials, **When** the native backend connects, **Then** it ends in `Failed { Authentication }` and never reaches `Connected`.

### Edge Cases
- Tunnel upgrade returns a non-200/201 status → terminal failure (network), no `Connected`.
- Encapsulation magic ≠ `0xf500` → frame dropped, not a crash.
- IPCP never converges → deterministic timeout → failure (no hang).
- Disconnect sends LCP Terminate-Request and the logout HTTP request; idempotent.

## Requirements *(mandatory)*

### Functional Requirements
- **FR-001**: Provide a pure-Rust F5 **framing** codec: F5 `0xf500|len16` encap encode/decode (incl. concatenated frames) and the RFC1662 **HDLC** variant (escape/unescape + FCS16).
- **FR-002**: Provide a pure-Rust **PPP** layer: build/parse PPP headers and LCP/IPCP/IP6CP control packets, and a negotiation **state machine** that reaches a "network up" state with the negotiated IPv4 address and DNS servers.
- **FR-003**: Implement **DPD**: reply to LCP Echo-Request with Echo-Reply; ~~emit keepalive (LCP Discard-Request) hooks~~.
  - **Echo-Reply (responder): DONE** (`ppp.rs` Echo-Request → Echo-Reply).
  - **Proactive keepalive (LCP Discard-Request sender): WON'T DO.** Rationale:
    liveness is already handled at a higher layer by the in-process supervisor's
    HTTP **health check** (`HealthChecker` against `health_check_endpoint`), which
    detects a dead/silent tunnel and drives reconnection — making a redundant
    PPP-level keepalive unnecessary in akon's architecture. The real F5 appliance
    does not require the client to send Discard-Requests to stay connected (proven
    by sustained production sessions). Revisit only if a future appliance is found
    to idle-timeout PPP without app-layer traffic.
- **FR-004**: Implement the F5 **HTTP auth** logic: detect success via presence of both `MRHSession` and `F5_ST` cookies; parse the `auth_form` fields; post `username`/`password` url-encoded.
- **FR-005**: Implement F5 **config parsing**: profile XML `<params>` extraction and options XML extraction of `Session_ID`, `ur_Z`, `IPV4_0/IPV6_0`, `hdlc_framing`, `DNS<N>`, routes; require `ur_Z`+`Session_ID`+≥1 IP family.
- **FR-006**: Build the F5 **tunnel-upgrade** request `GET /myvpn?sess=&hdlc_framing=&ipv4=&ipv6=&Z=&hostname=<base64>` with Host/User-Agent and **no Cookie**, and require a 200/201 response, reading `X-VPN-client-IP`.
- **FR-007**: Define a **`Transport`** seam (async byte stream) so all socket I/O is abstracted; provide a real TLS-over-TCP transport and an in-memory test transport.
- **FR-008**: Implement **`NativeF5Backend`** implementing [`VpnBackend`], orchestrating auth → config → upgrade → PPP, emitting the backend-agnostic lifecycle events.
- **FR-009**: Implement native **teardown**: PPP Terminate-Request then the `vdesk/hangup.php3?hangup_error=1` logout; idempotent.
- **FR-010**: Extend the **test actors framework** with: an in-memory `Transport`, a **fake F5 server actor** (HTTP auth/config + `/myvpn` upgrade), and a **PPP peer actor** (ACK/NAK LCP/IPCP), sufficient to test all layers offline.
- **FR-011**: The native backend MUST be testable and tested **without** real network, real TLS endpoint, or root, using the framework.
- **FR-012**: Prove **behavioral equivalence** between `NativeF5Backend` and `SimulatedBackend` over the shared scenario suite.
- **FR-013**: The native backend MUST NOT change production defaults in this feature (it is added alongside the openconnect backend; switching the default is a separate, later decision). No CLI/behavior regression in release builds.
- **FR-014**: TUN device creation and OS routing are isolated behind a seam (`TunDevice`) so the protocol/orchestration is testable without root; a real TUN impl may be provided but is not required for the framework-validated layers.

### Key Entities
- **F5 Framing Codec**: encode/decode F5 encap + HDLC.
- **PPP Engine**: header + LCP/IPCP/IP6CP packets + negotiation state machine.
- **F5 Auth/Config**: cookie/form logic + XML parsers.
- **Transport (seam)**: async byte stream; real TLS + in-memory test impl.
- **TunDevice (seam)**: OS tunnel interface; real + no-op test impl.
- **NativeF5Backend**: `VpnBackend` orchestrator.
- **Fake F5 Server Actor / PPP Peer Actor**: framework additions acting as the oracle.

## Success Criteria *(mandatory)*

### Measurable Outcomes
- **SC-001**: All native-backend layer tests (framing, PPP, auth, config) and the end-to-end native-backend tests pass under a plain `cargo test`, with no real server, no root, no network impact.
- **SC-002**: Framing is byte-exact vs. the openconnect wire format for the covered cases (validated by explicit test vectors derived from `f5.c`/`ppp.c`).
- **SC-003**: The native backend reaches `Connected` with a server-assigned IP and tears down cleanly, entirely against the fake F5 server.
- **SC-004**: The native and simulated backends produce equivalent lifecycle timelines for the shared scenario (cross-backend equivalence).
- **SC-005**: Authentication failure and tunnel-upgrade failure are handled as terminal failures (never `Connected`), deterministically.
- **SC-006**: Release build behavior is unchanged; the native backend and framework additions add no runtime cost to the default binary (feature-gated where test-only).
