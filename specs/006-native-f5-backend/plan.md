# Implementation Plan: Native F5 VPN Backend

**Branch**: `006-native-f5-backend` | **Date**: 2026-06-21 | **Spec**: [spec.md](./spec.md)

## Summary

Implement a native, pure-Rust F5 BIG-IP SSL VPN client as a `NativeF5Backend` implementing the `VpnBackend` boundary from spec 005, replacing the openconnect delegation for the F5 protocol. Build it **layer by layer, test-first**, using the test actors framework (extended as needed) as the ground-truth oracle. F5 is PPP-over-HTTPS, so the layers are: framing codec, PPP/LCP/IPCP engine, HTTP auth + XML config, and an orchestrator over a `Transport` seam. Prove behavioral equivalence to the simulated backend via the existing cross-backend machinery.

## Technical Context

**Language/Version**: Rust 2021, MSRV 1.70
**Primary Dependencies**: tokio (io-util/net/time/sync), data-encoding (base64), existing crate deps. Lightweight hand-rolled XML parsing for the flat F5 options XML (no new XML crate). Real TLS transport may use `tokio` + the existing rustls (via reqwest's rustls) or a thin TLS — but TLS is behind the `Transport` seam and NOT required for the framework-validated layers.
**Storage**: N/A
**Testing**: `cargo test`; framework actors as oracle; byte-exact framing vectors from `f5.c`/`ppp.c`.
**Target Platform**: Linux
**Project Type**: single (akon-core library)
**Performance Goals**: Framing/PPP operate per-packet with no allocation surprises; tests run in ms (logical time).
**Constraints**: No real network/root in framework-validated tests; additive (no production default change); zero release cost for test-only code.
**Scale/Scope**: ~6-8 new modules under `akon-core/src/vpn/f5/`, framework extensions under `testkit/`.

## Constitution Check

- [x] **Security-First**: Credentials flow through `Credentials` and are posted over TLS by the real transport; never logged. No plaintext secrets persisted. Cookie values treated as secrets.
- [x] **Modular Architecture**: Strict layering — framing / ppp / auth / config / transport / backend — each with a single responsibility and explicit interfaces. Seams (`Transport`, `TunDevice`) isolate I/O.
- [x] **Test-Driven Development**: Every layer is built test-first against the framework; framing has byte-exact vectors; equivalence proven vs. ground truth.
- [x] **Observability**: Lifecycle events + tracing at each stage; no secrets in logs.
- [x] **CLI-First Interface**: No CLI change in this feature; backend added alongside openconnect. Default unchanged.
- [x] **Test Actors & Seam-Isolated Testing** (Constitution v1.1.0): All real I/O is behind the `Transport`/`TunDevice` seams; the native backend is validated offline against the in-memory `MemoryTransport` + fake `F5ServerActor` (which reuses the real framing/PPP codecs as ground truth); pure layers (framing/ppp/auth/config) have byte-exact/deterministic tests; the same scenario suite proves equivalence to the simulated backend; a bounded **real** TLS-over-TCP end-to-end test confirms the production path (and caught the TLS read-coalescing/`leftover` bug); test actors are gated behind `test-actors`/`cfg(test)`. This feature is the first application of Principle VI.

**Security-Critical Changes**:
- [x] Password transmission: posted via the real TLS transport (`--passwd` equivalent over HTTPS form). Reviewed.
- [ ] OAuth/OTP/keyring/config-parsing: unchanged.

## Project Structure

```
akon-core/src/vpn/
├── backend.rs                 # (005) VpnBackend boundary — unchanged
├── transport.rs               # NEW: Transport seam (async byte stream) + TunDevice seam
├── f5/                        # NEW: native F5 implementation
│   ├── mod.rs
│   ├── framing.rs             # F5 0xf500|len encap + HDLC (FCS16) — pure
│   ├── ppp.rs                 # PPP header + LCP/IPCP/IP6CP packets + state machine — pure
│   ├── auth.rs                # cookie/form success logic + credential POST building — pure
│   ├── config.rs              # profile/options XML parsing — pure
│   ├── http.rs                # minimal HTTP/1.1 request build + response parse over Transport
│   └── backend.rs             # NativeF5Backend: orchestrates, impl VpnBackend
└── testkit/                   # framework extensions (feature-gated)
    ├── transport.rs           # NEW: in-memory duplex Transport
    ├── f5_server_actor.rs     # NEW: fake F5 server (HTTP auth/config + /myvpn)
    └── ppp_peer.rs            # NEW: PPP peer actor (ACK/NAK LCP/IPCP, echo)

akon-core/tests/
├── f5_framing_tests.rs        # NEW (US1)
├── f5_ppp_tests.rs            # NEW (US2)
├── f5_auth_config_tests.rs    # NEW (US3)
└── native_f5_backend_tests.rs # NEW (US4, equivalence)
```

**Structure Decision**: The native F5 stack lives under `akon-core/src/vpn/f5/` as always-compiled production code (it is a real backend), while the fake-server/peer/in-memory-transport additions are test-only under `testkit/` (gated behind `test-actors`/`cfg(test)`). The `Transport` and `TunDevice` seams keep the protocol logic free of real I/O so the framework validates it. The real TLS transport and real TUN device are thin adapters that can be added/hardened without touching the validated protocol layers.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Hand-rolled minimal HTTP/XML | Avoid heavyweight deps; F5 options XML is flat and the HTTP exchange is simple | A full XML/HTTP crate adds dependency weight contrary to the "no required dependencies" goal |
