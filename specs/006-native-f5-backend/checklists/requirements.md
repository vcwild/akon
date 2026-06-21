# Specification Quality Checklist: Native F5 VPN Backend

**Purpose**: Validate specification completeness, implementation, and verification quality
**Created**: 2026-06-21
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details leak into the *spec* (user stories stay outcome-focused)
- [x] Focused on developer/user value and the openconnect-removal goal
- [x] Layered scope is clearly bounded (framing / ppp / auth / config / transport / backend)
- [x] All mandatory sections completed (User Scenarios, Requirements, Success Criteria)

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] All acceptance scenarios are defined (US1–US4)
- [x] Edge cases are identified (bad magic, non-200 upgrade, IPCP non-convergence, idempotent teardown)
- [x] Scope is clearly bounded (additive backend; no production default change)
- [x] Dependencies and assumptions identified (builds on spec 005 `VpnBackend` + actors framework)

## Feature Readiness

- [x] All 14 functional requirements (FR-001..FR-014) map to layers and tests
- [x] User scenarios cover primary flows (framing, PPP negotiation, auth+config, E2E equivalence)
- [x] Feature meets the measurable outcomes in Success Criteria (SC-001..SC-006)
- [x] No implementation details leak into the specification

## Validation Results

### Content Quality Assessment

- ✓ **Layered, seam-isolated design**: framing/ppp/auth/config are pure; `Transport`/`TunDevice` isolate I/O; `NativeF5Backend` orchestrates.
- ✓ **Value focused**: every user story ties back to safely replacing openconnect.
- ✓ **Sections complete**: spec, plan, data-model, contracts, quickstart, and this checklist are all populated.

### Requirement Completeness Assessment

- ✓ **Testable**: each FR is backed by unit or E2E tests against the framework oracle.
- ✓ **Measurable success**: SC-001..SC-006 are objectively verified by the test outcomes below.
- ✓ **Edge cases covered**: bad encap magic, truncated frames, HDLC FCS failure, non-200/201 upgrade, IPCP timeout, auth failure — all have deterministic terminal outcomes.

### Feature Readiness Assessment

- ✓ **FRs mapped**: framing (FR-001), PPP/DPD (FR-002/003), auth (FR-004), config (FR-005), tunnel upgrade (FR-006), `Transport` seam (FR-007), `NativeF5Backend` (FR-008), teardown (FR-009), framework extensions (FR-010), offline testability (FR-011), equivalence (FR-012), additive/no-default-change (FR-013), `TunDevice` seam (FR-014).
- ✓ **Primary flows covered**: the four prioritized user stories are each independently testable.
- ✓ **Now a functional VPN, not just a handshake**: FR-002 (PPP), FR-005/006 (config/tunnel), and FR-008 (orchestration) are implemented and tested, plus **new** end-to-end capabilities — a bidirectional **data-plane packet pump** (TUN ↔ F5 framing ↔ transport), a **real Linux TUN** device (`/dev/net/tun` ioctl + `ip` addr/route config), **graceful teardown** (FR-009: PPP Terminate-Request + `vdesk/hangup.php3` logout + transport close), a **production constructor** (`connect_from_config`: real TLS + real TUN from `VpnConfig`), and **CLI wiring** (`native_backend = true` feeding the keyring-generated PIN+OTP password into the in-process native client).

### Test Outcomes (actual)

Unit tests (pure layers), all passing:

- ✓ **framing**: 15 tests — F5 encap byte-exactness, empty/concatenated/truncated decode, bad magic, HDLC round-trip + escape + asyncmap, FCS16 known vector + good-FCS, corrupted/short FCS.
- ✓ **ppp**: 14 tests — LCP/IPCP build+parse round-trip, MRU/Magic + IPADDR/DNS options, missing-FF03 + 1-byte PFC proto tolerance, truncated/overlong rejection, **full negotiation to `Up`** (LCP ACK → IPCP NAK adopt 10.20.30.40 + DNS 8.8.8.8 → ACK), echo-reply DPD, terminate.
- ✓ **auth**: 11 tests — both-cookies authenticate + combined header, only-one-cookie not authenticated, MRHSession re-set, empty value clears, urlencoding of reserved/unreserved/`+=%`, `F5_ST` parse + reject garbage, cookie-pair extraction.
- ✓ **config**: 14 tests — profile `<params>` extraction (declaration/whitespace/non-VPN skip/entity decode/no-VPN error), options full document, domains + multi-route LAN, idle-timeout + DTLS, missing `ur_Z`/`Session_ID`/IP-family errors, bool/int forms, self-closing/whitespace scan, DNSSuffix-vs-DNS disambiguation.

Plus new auth-form parsing tests (real `auth_form` parse, hidden-field
preservation + username/PIN+OTP password fill, single-quote/attr-order
tolerance, no-form and substring-false-match guards) and DNS detection/args
tests (systemd-resolved-preferred detection, resolvconf/file fallbacks,
`resolvectl dns`/`domain` arg construction, `resolv.conf` rendering, no-op
applier).

Plus the remaining `akon-core` library unit tests (http, transport, testkit, tun, data-plane helpers, etc.):

- ✓ **144 total lib unit tests pass** (`cargo test -p akon-core --lib`).

End-to-end (spec 006, `--features test-actors`), all passing:

- ✓ **4 native_f5_backend E2E tests pass**:
  1. successful connect to **10.20.30.40** against the fake F5 server (SC-003);
  2. **auth failure** → `Failed { Authentication }`, never `Connected` (SC-005);
  3. **tunnel-rejected (403)** → `Failed { Network }`, never `Connected` (SC-005);
  4. **native-vs-simulated equivalence** → both reach `Connected` at 10.20.30.40 with the same terminal milestone (SC-004 / FR-012).
- ✓ **2 native_f5_real_tls tests pass** — the production path against a **real local TLS server** (caught/guards the TLS read-coalescing/`leftover` bug).
- ✓ **3 native_f5_dataplane tests pass** — (a) a packet injected "from the OS" round-trips through the data plane (TUN → encap → transport → server echo → decap → TUN) and the TUN is configured with the negotiated **10.20.30.40**; (b) **DNS application** — the negotiated servers/domains are applied via the `DnsApplier` seam; (c) `disconnect()` triggers a **graceful teardown** and emits `Disconnected`, bounded, no hang.

Now implemented (moved out of the remaining gaps): **OTP / multi-step
auth-form parsing** (username + PIN+OTP, hidden fields preserved, redirect/form
loop until `MRHSession`+`F5_ST`), **host DNS application on Fedora/Ubuntu**
(`systemd-resolved` via `resolvectl`, with `resolvconf`/`resolv.conf`
fallbacks), and **in-process reconnection** (`native_supervise`: health-check +
exponential-backoff loop honoring the `[reconnection]` policy). The only
remaining gaps are **DTLS (UDP) transport** (TLS-only today; `no_dtls = true`
is satisfied) and **validation against a real production F5 appliance**.

Quality gates:

- ✓ **clippy clean** in both profiles (dev and release), with the MSRV (Rust 1.70) respected.
- ✓ **full workspace builds**; the **binary-crate tests pass** (CLI wiring compiles and runs).
- ✓ **default build unaffected** — the testkit additions are feature-gated and the native backend is opt-in; release behavior unchanged (SC-006 / FR-013).
- ✓ **no hangs** — every wait is bounded (10s handshake / 5s PPP / 3s logout / logical test timeouts); transport/TUN drop yields EOF so the pump and actor loops terminate.

## Status

**Overall Status**: ✅ IMPLEMENTED & VERIFIED — functional opt-in VPN (not the default)

The native F5 backend is implemented layer-by-layer, validated by the test actors framework as ground truth, proven behaviorally equivalent to the simulated backend, and is now a **functional in-process VPN**: control plane + data-plane packet pump + real Linux TUN + graceful teardown + production `connect_from_config` constructor + CLI wiring (`native_backend = true`), with the production transport path covered by a real-TLS test. It remains **opt-in** (FR-013): openconnect is still the default.

**Now implemented**: multi-step / OTP-form auth-form parsing (username + PIN+OTP, hidden fields preserved), host DNS application on Fedora/Ubuntu (`systemd-resolved` via `resolvectl`, with `resolvconf`/`resolv.conf` fallbacks), and in-process reconnection honoring the `[reconnection]` policy (`native_supervise`).

**Remaining gaps** (tracked, not blocking the opt-in milestone): DTLS (UDP) transport (TLS-only today; `no_dtls = true` is satisfied); the direct `/etc/resolv.conf` fallback is best-effort and not restored on revert; and validation against a **real production F5 appliance** (only the fake F5 server + a real local TLS server have been exercised).

## Notes

- The fake F5 server actor uses the **real** `framing`/`ppp` modules, so the green E2E and equivalence tests exercise the genuine wire codec on both sides — not a mirror re-implementation.
- Framing is byte-exact vs. the openconnect wire format for the covered cases (vectors derived from `f5.c`/`ppp.c`).
- **openconnect remains the default backend** (FR-013): the native backend is added alongside it and enabled only via `native_backend = true` for `protocol = f5`; switching the default is a separate, later decision once the remaining gaps (DTLS and real-appliance validation) are closed.
- The data-plane pump and real `LinuxTun` sit behind the existing `Transport`/`TunDevice` seams, so they were added without disturbing the framework-validated protocol layers; the offline `FakeTun` (recording config + packets) keeps the data plane testable without root.
- The PPP engine is simplified for a lossless TLS transport (no retransmit timers); a future DTLS/UDP path would reintroduce them behind the same `Transport` seam without touching the validated layers.
