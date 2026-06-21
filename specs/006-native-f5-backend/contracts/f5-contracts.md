# Contracts: Native F5 Backend

**Feature**: 006-native-f5-backend
**Phase**: 1 - Design

## Overview

This document specifies the public API contracts each native F5 module must satisfy. The layers are pure and seam-isolated (`framing`, `ppp`, `auth`, `config`, `http`), the orchestrator (`backend`) implements the durable [`VpnBackend`] boundary from spec 005, and the I/O seams (`Transport`, `TunDevice`) plus the testkit additions (`MemoryTransport`, `F5ServerActor`) make the whole stack validatable offline. Each item lists its Signature, Purpose, Pre/Post-conditions, and Behavior.

---

## Framing (`f5/framing.rs`) — pure

### `f5_encap`

```rust
pub fn f5_encap(ppp_payload: &[u8]) -> Vec<u8>;
```

- **Purpose**: Encode a PPP payload into one F5 non-HDLC frame.
- **Pre**: `ppp_payload.len() <= u16::MAX`.
- **Post**: returns exactly `4 + ppp_payload.len()` bytes: `0xF5 0x00` + big-endian length + payload.
- **Behavior**: `f5_encap(&[0x21,0xAA,0xBB]) == [0xF5,0x00,0x00,0x03,0x21,0xAA,0xBB]`; empty payload yields `[0xF5,0x00,0x00,0x00]`.

### `f5_decap`

```rust
pub fn f5_decap(buf: &[u8]) -> Result<Vec<Vec<u8>>, F5Error>;
```

- **Purpose**: Decode zero or more concatenated F5 non-HDLC frames.
- **Pre**: none (empty buffer is valid).
- **Post**: recovered PPP payloads in order; empty buffer → empty `Vec`.
- **Errors**: `BadEncapMagic(magic)` if a header magic ≠ `0xf500`; `TruncatedFrame { needed, have }` for a partial header or a declared length exceeding the remaining buffer.
- **Behavior**: round-trips `f5_encap`; decodes two concatenated frames as `[a, b]`.

### `hdlc_frame` / `hdlc_deframe`

```rust
pub fn hdlc_frame(payload: &[u8], asyncmap: u32) -> Vec<u8>;
pub fn hdlc_deframe(frame: &[u8]) -> Result<Vec<u8>, F5Error>;
```

- **Purpose**: RFC1662 async-HDLC framing/deframing for the F5 HDLC variant.
- **Pre (deframe)**: `frame` contains an unescaped payload of ≥ 2 bytes (the FCS).
- **Post (frame)**: FCS16 computed over the *unescaped* payload, complemented, appended little-endian; the whole frame escaped per `asyncmap` and bracketed by `0x7e` flags. First and last byte are `0x7e`.
- **Post (deframe)**: leading flag optional; reads to the next `0x7e`, unescapes, verifies `fcs16(payload‖fcs) == PPPGOODFCS16`, returns the payload with the FCS removed.
- **Errors**: `HdlcFcsInvalid` if too short or the FCS check fails.
- **Behavior**: `hdlc_deframe(hdlc_frame(p, m)) == p`; `0x7e`/`0x7d` always escaped; control chars `< 0x20` escaped only when their `asyncmap` bit is set.

### `fcs16`

```rust
pub fn fcs16(data: &[u8]) -> u16;
```

- **Purpose**: Running RFC1662 FCS16 (reflected poly `0x8408`, init `PPPINITFCS16 = 0xffff`).
- **Post**: returns the *uncomplemented* running FCS; `fcs16(&[]) == 0xffff`; `fcs16(payload‖fcs_le) == PPPGOODFCS16` for a correct trailer.

---

## PPP (`f5/ppp.rs`) — pure

### `build_ncp_frame` / `parse_ppp_frame`

```rust
pub fn build_ncp_frame(pkt: &NcpPacket) -> Vec<u8>;
pub fn parse_ppp_frame(frame: &[u8]) -> Result<NcpPacket, F5Error>;
```

- **Purpose**: Encode/decode a PPP control frame.
- **Post (build)**: emits `FF 03` + full 2-byte proto + `code id length(be) <TLVs>`; the `length` field covers `code..end`; never applies PFC/ACFC on send.
- **Post (parse)**: tolerates an optional `FF 03` prefix and a 1-byte (odd, PFC) or 2-byte protocol field; options parsed as `type(1) len(1) value(len-2)`.
- **Errors**: `MalformedPpp(_)` for a frame too short for the proto/NCP header, a declared length `< 4` or exceeding the buffer, or a TLV that overruns.
- **Behavior**: `parse_ppp_frame(build_ncp_frame(&p)) == p` for all constructed packets.

### Constructors

```rust
pub fn lcp_config_request(id: u8, magic: u32, mru: u16) -> NcpPacket;
pub fn ipcp_config_request(id: u8, requested_ip: [u8; 4]) -> NcpPacket;
pub fn lcp_echo_reply(id: u8, magic: u32, data: &[u8]) -> NcpPacket;
pub fn lcp_terminate_request(id: u8) -> NcpPacket;
```

- **Post**: LCP CONFREQ carries `LCP_MRU` (be16) + `LCP_MAGIC` (4 bytes); IPCP CONFREQ carries `IPCP_IPADDR` + `IPCP_DNS1` + `IPCP_DNS2` (DNS sent as `0.0.0.0` to be NAK-offered); echo reply carries `magic ‖ data`; terminate request has no options.

### `PppNegotiator`

```rust
impl PppNegotiator {
    pub fn new() -> Self;                                  // phase Dead
    pub fn start(&mut self) -> Vec<Vec<u8>>;               // -> EstablishLcp
    pub fn on_frame(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, F5Error>;
    pub fn phase(&self) -> PppPhase;
    pub fn negotiated_ipv4(&self) -> Option<[u8; 4]>;
    pub fn dns_servers(&self) -> Vec<[u8; 4]>;
}
```

- **Purpose**: Deterministic LCP→IPCP negotiation to "network up".
- **Pre**: `start()` called once before `on_frame`.
- **Post**:
  - `start()` transitions `Dead → EstablishLcp` and returns one LCP CONFREQ frame.
  - Peer LCP CONFREQ → CONFACK out (echoing options); peer CONFACK of our request + our ACK sent → `OpenedLcp` then immediately `NetworkIpcp` with an IPCP CONFREQ emitted.
  - Peer IPCP CONFNAK → adopt offered IPv4 + DNS, resend IPCP CONFREQ with the adopted IP (new id).
  - Peer IPCP CONFREQ → CONFACK out; peer CONFACK of our request + our ACK sent → `Up`, with `negotiated_ipv4()` and `dns_servers()` populated.
  - LCP Echo-Request → Echo-Reply, no phase change.
  - Terminate-Request → Terminate-Ack, phase `Terminated`.
  - Unknown protocol (e.g. IP6CP) → empty output, no error.
- **Invariants**: phase advances monotonically toward `Up` (or `Terminated`); no panic on malformed input — errors surface as `F5Error`.

### `PppPhase`

```rust
pub enum PppPhase { Dead, EstablishLcp, OpenedLcp, NetworkIpcp, Up, Terminated }
```

- **Contract**: ordering respects the state diagram in `data-model.md`; `Up` is only entered when both directions of IPCP are ACKed; `Terminated` is reachable from any non-dead phase.

---

## Auth (`f5/auth.rs`) — pure

### `F5CookieJar`

```rust
impl F5CookieJar {
    pub fn new() -> Self;
    pub fn ingest_set_cookie(&mut self, header_value: &str);
    pub fn get(&self, name: &str) -> Option<&str>;
    pub fn is_authenticated(&self) -> bool;
    pub fn cookie_header(&self) -> Option<String>;
}
```

- **Purpose**: Track `Set-Cookie` values and report F5 auth success.
- **Post**: `is_authenticated()` is `true` **iff** both `MRHSession` and `F5_ST` are present; `cookie_header()` returns `"MRHSession=<v>; F5_ST=<v>"` when authenticated, else `None`.
- **Behavior**: only the first `name=value` pair of a `Set-Cookie` is significant; attributes (`path`, `secure`, …) ignored; an empty value **deletes** the cookie (so a re-set empty `F5_ST` revokes auth); `MRHSession` may be re-set repeatedly before auth completes.

### `build_login_body` / `parse_f5_st` / `extract_cookie_pair`

```rust
pub fn build_login_body(username: &str, password: &str) -> String;
pub fn parse_f5_st(value: &str) -> Option<(i64, i64)>;
pub fn extract_cookie_pair(header_value: &str, name: &str) -> Option<String>;
```

- **Post**: `build_login_body` → strict urlencoded `username=..&password=..`; unreserved `A-Za-z0-9-_.~` literal, everything else `%XX` upper-case hex, space as `%20`. `parse_f5_st` → `Some((start, dur))` from the 4th/5th `z`-separated integer fields, else `None`. `extract_cookie_pair` → value of the leading `name=value` pair or `None`.

---

## Config (`f5/config.rs`) — pure

### `parse_profile`

```rust
pub fn parse_profile(xml: &str) -> Result<String, F5Error>;
```

- **Purpose**: Extract the resource `<params>` text from the profile XML.
- **Post**: returns the first `<params>` text inside a `<favorites type="VPN">` block, XML-entity-decoded.
- **Errors**: `InvalidConfig` if there is no VPN favorites block with a `<params>` element. Skips non-VPN favorites; tolerates XML declarations/whitespace.

### `parse_options`

```rust
pub fn parse_options(xml: &str) -> Result<F5Options, F5Error>;
```

- **Purpose**: Parse the tunnel options XML into [`F5Options`].
- **Post**: populates `session_id`, `ur_z`, `ipv4`/`ipv6`/`hdlc_framing`/`dtls`/`default_gateway` (int or yes/on/no/off), `idle_timeout`, `dtls_port`, and the ordered families `dns` (`DNS<n>`), `domains` (`DNSSuffix<n>`), `routes` (`LAN<n>`, whitespace-split).
- **Errors**: `InvalidConfig` if `ur_Z` or `Session_ID` is missing, or neither `IPV4_0` nor `IPV6_0` is enabled (mirrors openconnect's `(*ipv4 < 1 && *ipv6 < 1) || !*ur_z || !*session_id`).
- **Behavior**: `DNSSuffix<n>` is never mis-captured as a DNS server.

---

## HTTP (`f5/http.rs`) — minimal HTTP/1.1 over `Transport`

### `HttpRequest` / `HttpResponse`

```rust
impl<'a> HttpRequest<'a> {
    pub fn get(path: &'a str, host: &'a str) -> Self;
    pub fn post_form(path: &'a str, host: &'a str, body: String) -> Self;
    pub fn with_header(self, name: &str, value: &str) -> Self;
    pub fn to_bytes(&self) -> Vec<u8>;
}
impl HttpResponse {
    pub fn header_all(&self, name: &str) -> Vec<&str>;   // case-insensitive
    pub fn header(&self, name: &str) -> Option<&str>;
}
```

- **Post (to_bytes)**: emits the request line, `Host`, `User-Agent: akon-native-f5/1.0`, `Connection: keep-alive`, any extra headers, and a `Content-Length` when a body is present; `post_form` adds `Content-Type: application/x-www-form-urlencoded`.
- **Post (response)**: header names lowercased; `header_all("set-cookie")` returns every value in order.

### `send_request` / `read_response`

```rust
pub async fn send_request<T: Transport + ?Sized>(
    transport: &mut T, request: &HttpRequest<'_>,
) -> Result<HttpResponse, F5Error>;
pub async fn read_response<T: Transport + ?Sized>(
    transport: &mut T,
) -> Result<HttpResponse, F5Error>;
```

- **Purpose**: Drive one request/response over the transport seam.
- **Post**: reads the header block (`\r\n\r\n`), parses status + headers, then reads a `Content-Length`-delimited body (truncated to the declared length, leaving trailing bytes — e.g. the PPP stream after `/myvpn` — unconsumed).
- **Errors**: `MalformedHttp(_)` on send/recv failure, premature close before headers, or an unparseable status line. Tolerates responses split across multiple reads.

---

## Transport / TunDevice seams (`vpn/transport.rs`)

### `Transport`

```rust
#[async_trait]
pub trait Transport: Send {
    async fn send(&mut self, data: &[u8]) -> io::Result<()>;
    async fn recv(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    async fn close(&mut self) -> io::Result<()> { Ok(()) }
}
```

- **Contract**: a reliable, ordered, bidirectional byte stream. `send` writes all bytes. `recv` returns `Ok(0)` **iff** the peer has closed (EOF). `close` is idempotent. No message framing is implied — PPP/HTTP framing lives above.

### `TunDevice` / `TunConfig`

```rust
#[async_trait]
pub trait TunDevice: Send {
    async fn configure(&mut self, config: &TunConfig) -> io::Result<()>;
    async fn write_packet(&mut self, packet: &[u8]) -> io::Result<()>;
    async fn read_packet(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}
```

- **Contract**: OS tunnel seam. `configure` applies the negotiated `TunConfig` (ipv4/mtu/dns/domains/routes). `read_packet` returns `Ok(0)` when the device closes. Production needs `CAP_NET_ADMIN`; the test fake requires no root, so orchestration is validated without privileges (FR-014).

---

## Backend (`f5/backend.rs`) — implements `VpnBackend`

### `NativeF5Backend`

```rust
impl NativeF5Backend {
    pub fn with_transport(transport: Box<dyn Transport>, host: impl Into<String>) -> Self;
}
impl VpnBackend for NativeF5Backend {
    fn connect(&mut self, credentials: Credentials)
        -> Result<UnboundedReceiver<LifecycleEvent>, BackendError>;
    fn disconnect(&mut self) -> Result<(), BackendError>;
    fn is_alive(&self) -> bool;
    fn handle(&self) -> Option<ConnectionHandle>;
}
```

- **Purpose**: Orchestrate auth → config → tunnel upgrade → PPP and emit the backend-agnostic lifecycle.
- **Pre**: `with_transport` supplies a connected transport; `connect` consumes it (a second `connect` returns `StartFailed`).
- **Post (success)**: stream emits `Connecting → Authenticating → SessionEstablished → LinkUp → Connected { ip, device }`; `is_alive() == true`; `handle().is_some()`.
- **Post (failure)**: stream ends in `Failed { kind, detail }` and never emits `Connected`; `is_alive() == false`. Mapping: `AuthFailed → Authentication`, `InvalidConfig → Backend`, framing/PPP/HTTP/`TunnelUpgradeRejected → Network`; outer 10s timeout → `Failed { Network, "handshake timed out" }`.
- **Invariants**: no openconnect/sudo/child process is spawned for the protocol; the whole handshake is bounded (10s outer, 5s PPP) so it cannot hang. `disconnect` is idempotent (`is_alive()`/`handle()` cleared, no-op success if already down). The `/myvpn` request carries **no** Cookie (auth via `sess` + `Z` query params).

---

## Testkit additions (test-only)

### `MemoryTransport` (`testkit/transport.rs`)

```rust
impl MemoryTransport {
    pub fn pair() -> (MemoryTransport, MemoryTransport);
}
impl Transport for MemoryTransport { /* send / recv / close */ }
```

- **Contract**: `pair()` returns two connected endpoints; bytes written to one are readable from the other, in order. `close()` **and** `Drop` flip a synchronous `closed` flag and wake any blocked `recv`, which then returns `Ok(0)` — guaranteeing actor loops terminate on disconnect instead of hanging. No real socket, TLS, or network.

### `F5ServerActor` / `F5ServerScript` (`testkit/f5_server_actor.rs`)

```rust
impl F5ServerScript {
    pub fn default() -> Self;            // successful session, IP 10.20.30.40, DNS 8.8.8.8
    pub fn auth_failure() -> Self;       // accept_auth = false
    pub fn tunnel_rejected(status: u16) -> Self;
}
impl F5ServerActor {
    pub fn new(script: F5ServerScript) -> Self;
    pub async fn run<T: Transport + ?Sized>(&self, transport: &mut T);
}
```

- **Contract**: `run` plays the scripted F5 server over the transport — login form, cookie-setting credential POST (or rejection), profile/options XML, `/myvpn` with `tunnel_status` + `X-VPN-client-IP`, then the PPP peer (ACK LCP, NAK-then-ACK IPCP, gateway request) **using the real `framing`/`ppp` modules**. Returns when the exchange completes or the transport closes. Performs no real I/O and needs no root/network. This is the ground-truth oracle: a passing test exercises the genuine codec, not a mirror.

---

## Testing Contracts

The four end-to-end tests in `akon-core/tests/native_f5_backend_tests.rs` (gated on `feature = "test-actors"`) prove:

1. **`native_f5_reaches_connected_against_fake_server`** — against `F5ServerScript::default()`, the native backend's timeline contains `Connecting`, `Authenticating`, `SessionEstablished`, and `Connected`; the `Connected` IP is the server-assigned `10.20.30.40`; `is_alive()` is true and `handle()` is `Some`. (SC-003: full offline connect.)
2. **`native_f5_auth_failure_never_connects`** — against `F5ServerScript::auth_failure()`, the timeline never contains `Connected` and ends in `Failed { Authentication }`; `is_alive()` is false. (SC-005: auth failure is terminal.)
3. **`native_f5_tunnel_rejected_fails`** — against `F5ServerScript::tunnel_rejected(403)`, the timeline never contains `Connected` and ends in `Failed { Network }`. (SC-005: tunnel-upgrade failure is terminal.)
4. **`native_and_simulated_backends_are_equivalent`** — the same successful scenario run against `NativeF5Backend` (vs. the fake F5 server) and `SimulatedBackend` (vs. `VpnServerActor::successful_connect`) both reach `Connected` with the same IP (`10.20.30.40`) and the same terminal milestone, demonstrating the native backend is a behaviorally-equivalent drop-in. (SC-004 / FR-012: cross-backend equivalence.)

All four run under a plain `cargo test` with no real server, no root, no network impact, and complete without hanging (logical timeouts bound every wait).

## Backward Compatibility

The native backend is **additive**: it is added alongside `OpenConnectBackend` and does not change the production default in this feature (FR-013). No CLI or release-build behavior regresses. The testkit additions (`MemoryTransport`, `F5ServerActor`) are test-only and add no runtime cost to the default binary.

## Summary

`framing`, `ppp`, `auth`, and `config` are pure, byte-/value-exact contracts; `http` is a minimal client over the `Transport` seam; `NativeF5Backend` composes them into the durable `VpnBackend` boundary with deterministic, bounded, failure-safe behavior. The `Transport`/`TunDevice` seams plus the `MemoryTransport` + `F5ServerActor` oracle make every contract above verifiable entirely offline — the foundation for safely replacing openconnect.
