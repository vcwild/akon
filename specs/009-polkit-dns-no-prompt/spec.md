# Feature Specification: No password prompts during `akon vpn on` (DNS via polkit)

**Feature Branch**: `009-polkit-dns-no-prompt`
**Created**: 2026-06-22
**Status**: Draft
**Input**: User description: "when running akon vpn on we are prompted multiple
times to type the user password to add the network connections. the user
shouldn't be required to type anything or be prompted."

## Overview

Since akon became a rootless native client (runs as the user with only a
`CAP_NET_ADMIN` file capability), applying the tunnel's DNS via
`resolvectl`/systemd-resolved triggers **polkit authentication prompts**.
systemd-resolved gates `set-dns-servers`, `set-domains`, and `set-default-route`
behind `auth_admin` (and `auth_admin_keep`), so an unprivileged user is asked to
authenticate. akon makes three such calls per connect, so the user sees **three
password prompts** ("authentication required to … network …") on every
`akon vpn on`, plus more on disconnect.

A VPN client must connect without interactive authentication prompts. The fix is
to ship a **polkit rule** that grants exactly the resolve1 DNS actions akon needs
to **local, active user sessions, without authentication** — the same mechanism
used by other VPN/network tools. The rule is installed by akon's packaging (deb,
rpm) and by `make install`, alongside the existing `setcap` capability grant.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Connect without any prompt (Priority: P1)

As a user, when I run `akon vpn on` I am never asked to type my password or to
authenticate. The VPN connects, applies its DNS, and (in background mode) returns
my prompt — with zero interactive prompts.

**Why this priority**: This is the reported defect. Repeated password prompts on
every connect are a severe UX regression and make background mode and lazy mode
unusable (a prompt blocks the flow).

**Independent Test**: After the polkit rule is installed, running the DNS-apply
step as an unprivileged user completes successfully without any polkit agent
prompt. Testable by asserting the `resolvectl` calls succeed non-interactively
(exit 0, no agent invocation) when the rule is present, and documenting the
behaviour without it.

**Acceptance Scenarios**:

1. **Given** akon is installed (with its polkit rule), **When** I run
   `akon vpn on`, **Then** the connection completes with no password prompt and
   the VPN DNS is applied to the tunnel link.
2. **Given** background mode, **When** I run `akon vpn on`, **Then** the terminal
   returns with no prompt at any point.
3. **Given** I disconnect with `akon vpn off`, **Then** the DNS revert also
   happens without a prompt.

### User Story 2 — Least-privilege, scoped grant (Priority: P2)

As a security-conscious operator, the no-auth grant is limited to exactly the
resolve1 DNS actions akon needs, for local active sessions only — not a blanket
admin bypass.

**Why this priority**: Granting "no authentication" must be scoped so it cannot
be abused. We grant only `set-dns-servers`, `set-domains`, `set-default-route`,
and `revert` for resolve1 — nothing else.

**Independent Test**: Inspect the installed polkit rule: it returns `YES` only
for the listed resolve1 action ids and only for active local sessions; all other
actions are unaffected (fall through to system defaults).

**Acceptance Scenarios**:

1. **Given** the installed rule, **When** a non-listed privileged action is
   attempted, **Then** the rule does not affect it (default polkit behaviour).
2. **Given** a remote/inactive session, **When** a listed action is attempted,
   **Then** the rule does not grant it unconditionally (active sessions only).

### Edge Cases

- Host has **no polkit** (e.g. minimal/container) → resolvectl behaviour is
  unchanged; akon's DNS apply remains best-effort and must not hard-fail the
  connection if DNS can't be set (names may not resolve, but the tunnel is up).
- Host uses **resolvconf** or **/etc/resolv.conf** instead of systemd-resolved →
  no polkit involved; unaffected.
- Polkit rule installed but **polkitd not reloaded** → document that the rule
  takes effect for new sessions / after polkit reload; package post-install
  should not require a reboot.
- akon run as **root** (e.g. via sudo for the data-plane soak) → no prompt
  regardless (root bypasses polkit); the rule is for the rootless path.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `akon vpn on` MUST NOT produce any interactive authentication
  prompt during normal operation as an unprivileged user.
- **FR-002**: The project MUST ship a polkit rule granting, without
  authentication, the resolve1 actions akon uses to apply/revert tunnel DNS:
  `org.freedesktop.resolve1.set-dns-servers`,
  `org.freedesktop.resolve1.set-domains`,
  `org.freedesktop.resolve1.set-default-route`, and
  `org.freedesktop.resolve1.revert`.
- **FR-003**: The grant MUST be scoped to **local active** sessions (not remote,
  not inactive) and MUST NOT affect any action outside the listed set.
- **FR-004**: The polkit rule MUST be installed by the deb and rpm packages and
  by `make install`, and removed on uninstall.
- **FR-005**: DNS application MUST remain **best-effort**: if it fails (e.g. no
  polkit, no resolved, rule absent), `akon vpn on` MUST still establish the
  tunnel and surface a warning — it MUST NOT hard-fail or hang on a prompt in a
  non-interactive context.
- **FR-006**: When running non-interactively (no polkit agent / no tty), the DNS
  calls MUST NOT block waiting for authentication; they fail fast and akon
  continues (the rule makes them succeed; without it they fail rather than hang).
- **FR-007**: The fix MUST NOT require the user to run `akon vpn on` with `sudo`.

### Key Entities

- **Polkit rule**: a JavaScript `.rules` file (or `.pkla`) installed under the
  system polkit rules directory that returns `polkit.Result.YES` for the listed
  resolve1 actions when the subject is local + active.
- **DNS applier**: the existing `SystemDnsApplier` that invokes `resolvectl`; its
  behaviour is unchanged except it now succeeds without prompting.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With akon installed, `akon vpn on` completes with **zero** password
  prompts for an unprivileged user.
- **SC-002**: The tunnel link's DNS is correctly applied (VPN-only names resolve)
  after connect, without prompting.
- **SC-003**: The polkit grant is limited to the four listed resolve1 actions and
  local active sessions; verifiable by inspecting the installed rule.
- **SC-004**: Uninstalling akon removes the polkit rule.
- **SC-005**: On a host without polkit/resolved, `akon vpn on` still connects
  (DNS best-effort) and never hangs on a prompt.
