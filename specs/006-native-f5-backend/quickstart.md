# Quickstart: Native F5 VPN Backend

**Feature**: 006-native-f5-backend
**Date**: 2026-06-21
**For**: Developers working on the native F5 (openconnect-replacement) backend

## 🎯 What You're Building

A **native, in-process F5 BIG-IP SSL VPN client** in pure Rust — the replacement for the `sudo openconnect` delegation, for the F5 protocol. F5 is **PPP-over-HTTPS**: HTTP auth → XML config → an HTTP "tunnel upgrade" → a PPP (LCP/IPCP) session framed with an F5-specific 4-byte encapsulation. `NativeF5Backend` implements the same `VpnBackend` trait as the simulated and openconnect backends, so the existing test actors framework (spec 005) proves it behaves identically — **with no real server, no root, and no network**.

## 📋 Quick Context

**Why**: openconnect is the dependency we want to remove. Spec 005 built the backend-agnostic boundary + actors framework to make removal safe; this feature delivers the replacement and proves it equivalent to ground truth.

**How it's validated**: a fake F5 server actor speaks the real wire protocol over an in-memory transport, using the *real* framing/PPP code. If the native backend reaches `Connected` against it — and matches the simulated backend's lifecycle — the replacement is correct.

**Status**: implemented, all tests pass. The openconnect backend remains the **default**; switching is a later, separate decision (FR-013).

## 🛠️ How It's Wired

The native F5 stack lives under `akon-core/src/vpn/f5/`, layered bottom-up, with all I/O behind seams:

```
akon-core/src/vpn/
├── transport.rs          # Transport + TunDevice seams (async byte stream / OS tunnel)
├── backend.rs            # (005) VpnBackend boundary + LifecycleEvent — unchanged
├── f5/
│   ├── mod.rs            # F5Error; re-exports NativeF5Backend
│   ├── framing.rs        # ⬇ f5_encap / f5_decap / hdlc_frame / hdlc_deframe / fcs16  (pure)
│   ├── ppp.rs            # ⬆ NcpPacket / PppNegotiator / PppPhase  (pure)
│   ├── auth.rs           #   F5CookieJar / build_login_body  (pure)
│   ├── config.rs         #   F5Options / parse_profile / parse_options  (pure)
│   ├── http.rs           #   HttpRequest / send_request over Transport
│   └── backend.rs        # 🎯 NativeF5Backend: auth → config → /myvpn → PPP, impl VpnBackend
└── testkit/              # test-only (feature "test-actors")
    ├── transport.rs      # MemoryTransport::pair() — in-memory duplex
    └── f5_server_actor.rs# F5ServerActor / F5ServerScript — the ground-truth oracle
```

Data flows up the layers: `backend` calls `http`/`auth`/`config` for the HTTP phase, then drives `ppp` over `framing` over a `Transport`. In production the `Transport` is TLS-over-TCP; in tests it's a `MemoryTransport` connected to the `F5ServerActor`.

## 🧪 Testing It Offline

The whole F5 protocol runs without a server, without root, and without touching the network. Wire `NativeF5Backend::with_transport` to one end of a `MemoryTransport` pair and let `F5ServerActor` drive the other — exactly like the test helper in `native_f5_backend_tests.rs`:

```rust
use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::testkit::transport::MemoryTransport;
use std::time::Duration;

#[tokio::test]
async fn native_f5_connects_offline() {
    // 1. In-memory duplex: client end for the backend, server end for the actor.
    let (client, mut server) = MemoryTransport::pair();

    // 2. Spawn the fake F5 server (default script = successful session,
    //    assigns 10.20.30.40, DNS 8.8.8.8) using the REAL framing/ppp codec.
    tokio::spawn(async move {
        F5ServerActor::new(F5ServerScript::default())
            .run(&mut server)
            .await;
        // dropping `server` here closes the transport -> backend sees EOF
    });

    // 3. Native backend over the client end — no TLS, no sudo, no /dev/net/tun.
    let mut backend = NativeF5Backend::with_transport(Box::new(client), "vpn.example.com");
    let mut rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");

    // 4. Collect lifecycle events until a terminal one.
    let mut connected_ip = None;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
        if let LifecycleEvent::Connected { ip, .. } = &ev {
            connected_ip = Some(ip.to_string());
            break;
        }
        if matches!(ev, LifecycleEvent::Failed { .. }) {
            panic!("unexpected failure: {ev:?}");
        }
    }

    // 5. It reached Connected with the server-assigned IP — entirely offline.
    assert_eq!(connected_ip.as_deref(), Some("10.20.30.40"));
    assert!(backend.is_alive());
    assert!(backend.handle().is_some());
}
```

Run it:

```bash
cargo test -p akon-core --features test-actors native_f5
```

To exercise the failure arcs, swap the script: `F5ServerScript::auth_failure()` (ends in `Failed { Authentication }`, never `Connected`) or `F5ServerScript::tunnel_rejected(403)` (ends in `Failed { Network }`).

## ✅ Definition of Done

- [x] `framing` byte-exact codec: `f5_encap`/`f5_decap` (incl. concatenated frames) + `hdlc_frame`/`hdlc_deframe` + `fcs16`
- [x] `ppp` engine: `NcpPacket`/`NcpOption` build/parse + `PppNegotiator` reaching `PppPhase::Up` with negotiated IP + DNS; DPD echo reply; terminate
- [x] `auth` logic: `F5CookieJar` requiring both `MRHSession` + `F5_ST`; urlencoded credential body
- [x] `config` parsing: `parse_profile` + `parse_options` requiring `ur_Z` + `Session_ID` + an IP family
- [x] `http` over the `Transport` seam (Content-Length bodies, multiple `Set-Cookie`)
- [x] `NativeF5Backend` orchestrates auth → config → `/myvpn` → PPP and implements `VpnBackend`
- [x] `Transport` / `TunDevice` seams isolate all I/O; `MemoryTransport` + `F5ServerActor` enable offline tests
- [x] E2E: reaches `Connected` with assigned IP; auth failure + tunnel-rejected are terminal; native ≡ simulated
- [x] All tests pass under `cargo test`; clippy clean; default build unaffected; no hangs
- [x] openconnect remains the default backend (FR-013)

## 📚 Key Files Reference

| File | Purpose |
|------|---------|
| `src/vpn/f5/mod.rs` | `F5Error`; module layout; re-exports `NativeF5Backend` |
| `src/vpn/f5/framing.rs` | F5 encap + HDLC codec (`f5_encap`/`f5_decap`/`hdlc_frame`/`hdlc_deframe`/`fcs16`) |
| `src/vpn/f5/ppp.rs` | PPP packets + `PppNegotiator` + `PppPhase` state machine |
| `src/vpn/f5/auth.rs` | `F5CookieJar`, `build_login_body`, `parse_f5_st` |
| `src/vpn/f5/config.rs` | `F5Options`, `parse_profile`, `parse_options` |
| `src/vpn/f5/http.rs` | `HttpRequest`/`HttpResponse`, `send_request`/`read_response` |
| `src/vpn/f5/backend.rs` | `NativeF5Backend`, `run_session`, `run_ppp`, `build_myvpn_path` |
| `src/vpn/transport.rs` | `Transport` + `TunDevice` seams, `TunConfig` |
| `src/vpn/testkit/transport.rs` | `MemoryTransport::pair()` in-memory duplex |
| `src/vpn/testkit/f5_server_actor.rs` | `F5ServerActor` / `F5ServerScript` ground-truth oracle |
| `tests/native_f5_backend_tests.rs` | The 4 E2E tests (connect, auth fail, tunnel reject, equivalence) |

## 💡 Tips & Gotchas

1. **Framing is byte-exact vs. openconnect.** `f5_encap` emits `0xF5 0x00 | len16 | payload`; HDLC uses RFC1662 `0x7e`/`0x7d` escaping with the little-endian FCS16 (`PPPGOODFCS16 = 0xf0b8`). The vectors in `framing.rs` are derived from `f5.c`/`ppp.c` — change them only with a matching openconnect reference.

2. **Transport drop = EOF.** Dropping (or `close`-ing) a `MemoryTransport` endpoint flips a synchronous `closed` flag and wakes any blocked `recv`, which returns `Ok(0)`. This is what makes the `F5ServerActor` loop (and the backend's PPP loop) terminate deterministically instead of hanging. Let the server task drop its endpoint to end the session cleanly.

3. **The same scenario suite proves equivalence.** `native_and_simulated_backends_are_equivalent` runs the identical successful connect against both `NativeF5Backend` and `SimulatedBackend` and asserts the same `Connected` IP and terminal milestone — that's the migration guarantee, not a separate codepath.

4. **The fake server uses the real codec.** `F5ServerActor` calls into `f5::framing` and `f5::ppp` directly, so a green equivalence test exercises the genuine wire code on both sides — it is not a mirror re-implementation.

5. **openconnect is still the default.** This backend is additive (FR-013). Don't wire it into the CLI default in this feature; switching is a later decision once it's hardened (real TLS transport + real TUN device behind the existing seams).

6. **Everything is bounded.** The handshake has a 10s outer timeout and PPP a 5s inner deadline, so a misbehaving peer fails deterministically (`Failed { Network, "…timed out" }`) rather than hanging.

## 🔗 Related Documentation

- [Feature Spec](./spec.md) — requirements, user stories, success criteria
- [Implementation Plan](./plan.md) — architecture & module layout
- [Data Model](./data-model.md) — entities, sequence + state diagrams
- [F5 Contracts](./contracts/f5-contracts.md) — per-module API contracts
- [Spec 005 — Test Actors Framework](../005-test-actors-framework/spec.md) — the `VpnBackend` boundary + actors this builds on

---

**Ready to dig in?** Start at `framing.rs` (the foundation), follow it up through `ppp.rs` to `backend.rs`, then read `native_f5_backend_tests.rs` to see the whole thing proven offline. 🚀
