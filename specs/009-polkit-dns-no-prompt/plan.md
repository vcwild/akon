# Implementation Plan: No password prompts during `akon vpn on` (DNS via polkit)

**Branch**: `009-polkit-dns-no-prompt` | **Date**: 2026-06-22 | **Spec**: [spec.md](./spec.md)

## Summary

Ship a polkit rule that lets a local, active, unprivileged user apply/revert the
tunnel's DNS through systemd-resolved **without authentication**, eliminating the
3 password prompts per `akon vpn on`. The rule is a modern polkit JavaScript
`.rules` file installed by the deb/rpm packages and `make install`. Additionally,
harden the DNS applier so resolvectl calls **never block on a prompt** in a
non-interactive context (fail fast, best-effort) — so even without the rule akon
connects rather than hanging.

## Technical Context

**Mechanism**: polkit ≥ 0.106 JavaScript rules (current Fedora/Ubuntu ship
polkit 12x). Rule file: `49-akon-resolved-dns.rules`.
**Install locations**:
- Package-provided rule → `/usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules`
- `make install` (local) → same path (needs root, like setcap).
**Scope**: returns `polkit.Result.YES` for the four resolve1 DNS actions when
`subject.local && subject.active`; otherwise no opinion (falls through).
**No new Rust dependencies**. Optional small hardening to `dns.rs` (timeout /
non-interactive env) so calls fail fast without a prompt agent.
**Testing**: the rule is a static asset (lint its JS syntax + content via a unit
test that checks the action ids and the local/active guard). The applier
hardening is covered by existing DNS tests; a doc/quickstart explains manual
verification.
**Platform**: Linux + systemd-resolved + polkit. Other DNS backends unaffected.

## Constitution Check

- [x] **Security-First**: The grant is **least-privilege** — only the four
  resolve1 DNS actions, only for local active sessions. No blanket admin bypass.
  No credentials involved. The rule is auditable (a short static file).
- [x] **Modular Architecture**: The rule is a packaging artifact; the DNS applier
  change is isolated to `dns.rs`. No cross-module coupling.
- [x] **Test-Driven Development**: A unit test asserts the shipped rule contains
  exactly the intended action ids and the `local && active` guard, and excludes
  anything else. Applier hardening covered by existing/extended DNS tests.
- [x] **Observability**: DNS apply already logs (gated). A clear WARN is emitted
  if DNS can't be applied (best-effort), so failures are visible without prompts.
- [x] **CLI-First Interface**: No CLI change; `akon vpn on` simply stops
  prompting.
- [x] **Test Actors & Seam-Isolated Testing**: DNS is already behind the
  `DnsApplier` seam; tests use `NoopDns`/`FakeDns` and never touch the host
  resolver. The polkit rule's correctness is validated by a static-content test
  (no live polkit needed). A human verifies the no-prompt behaviour on a real
  desktop per quickstart.

**Security-Critical Changes**:
- [x] Configuration parsing: N/A.
- [x] The polkit grant is reviewed: scoped to resolve1 DNS actions + local active
  sessions only.

## Design Decisions

1. **Polkit JS `.rules`, not `.pkla`.** Targets modern polkit; `.pkla` (pklocalauthority)
   is deprecated/removed in current distros. The rule:
   ```javascript
   polkit.addRule(function(action, subject) {
       if (subject.local && subject.active &&
           (action.id == "org.freedesktop.resolve1.set-dns-servers" ||
            action.id == "org.freedesktop.resolve1.set-domains" ||
            action.id == "org.freedesktop.resolve1.set-default-route" ||
            action.id == "org.freedesktop.resolve1.revert")) {
           return polkit.Result.YES;
       }
   });
   ```
   File prefix `49-` so it sorts before the distro `50-default.rules`.

2. **Scope by `subject.local && subject.active`**, not by user/group — keeps it
   simple and safe (a logged-in desktop user), matching how VPN tools grant
   network actions. (A future refinement could gate on a unix group like
   `akon`/`netdev`; out of scope here.)

3. **Best-effort, non-blocking DNS apply (defense in depth).** Ensure resolvectl
   invocations don't hang on a polkit agent when none can authorize: rely on the
   rule for success; if a call fails, log a WARN and continue (tunnel stays up).
   The existing code already treats domain/default-route as best-effort; ensure
   the primary `set-dns` failure also degrades gracefully rather than aborting
   the data plane (it currently returns Err — downgrade to WARN + continue).

4. **Installed by packaging + make install + removed on uninstall.** Mirrors the
   `setcap` grant lifecycle already added in v2.0.0.

## Project Structure

```
packaging/polkit/
└── 49-akon-resolved-dns.rules      # NEW: the polkit rule (shipped asset)

Cargo.toml                          # MODIFY: deb/rpm assets include the rule
debian/postinst, rpm/post-install.sh # MODIFY: (rule is a static asset; ensure path)
debian/postrm,  rpm/post-uninstall.sh # MODIFY: remove the rule on uninstall
Makefile                            # MODIFY: install/uninstall the rule

akon-core/src/vpn/f5/dns.rs         # MODIFY: set-dns failure -> WARN + continue
akon-core/tests/ or dns.rs tests    # ADD: static-content test for the rule
```

## Complexity Tracking

No constitution violations. The polkit rule is a minimal, auditable static asset.
