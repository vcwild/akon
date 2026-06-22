# ADR 0003 — Ship a polkit rule for rootless DNS (no password prompts)

* Status: Accepted
* Deciders: akon maintainers
* Date: 2026-06-22
* Related: ADR 0001 (rootless netlink), ADR 0002 (native-only backend), spec 009

## Context

Since v2.0.0 akon runs rootless: it operates as the user with only a
`CAP_NET_ADMIN` file capability. Applying the VPN tunnel's DNS goes through
systemd-resolved via `resolvectl` (`set-dns-servers`, `set-domains`,
`set-default-route`, and `revert` on teardown).

systemd-resolved gates those actions behind polkit with `auth_admin` /
`auth_admin_keep` defaults. An unprivileged user therefore gets an
**authentication prompt for each call** — three prompts on every `akon vpn on`,
plus more on disconnect. This breaks the UX, and makes background mode and lazy
mode unusable (a prompt blocks the flow). The previous (openconnect) design never
hit this because everything ran as root via sudo.

`CAP_NET_ADMIN` does not help: polkit authorization is independent of process
capabilities. We need polkit itself to authorize these specific actions for the
local user.

Alternatives considered:
- **Run `akon vpn on` under sudo/root** — regresses the rootless model and
  re-introduces the sudo dependency we removed in v2.0.0.
- **Write `/etc/resolv.conf` directly** — requires root, conflicts with
  systemd-resolved's management, and is fragile.
- **Per-user manual polkit configuration** — works, but every user would have to
  do it; not shippable as a product default.

## Decision

**Ship a scoped polkit rule** (`packaging/polkit/49-akon-resolved-dns.rules`)
that returns `polkit.Result.YES` for exactly the four resolve1 DNS actions akon
uses — `set-dns-servers`, `set-domains`, `set-default-route`, `revert` — and only
when the subject is a **local, active** session. It is installed to
`/usr/share/polkit-1/rules.d/` by the deb/rpm packages and by `make install`,
and removed on uninstall. It uses modern polkit JavaScript rules (polkit ≥
0.106), which all current Fedora/Ubuntu releases ship.

DNS application remains **best-effort**: if the rule is absent (or there is no
polkit/resolved), the call fails fast (polkit denies non-interactively rather
than hanging) and akon keeps the tunnel up with a visible warning.

## Consequences

- **No prompts**: `akon vpn on` (incl. background and lazy mode) connects and
  applies DNS with zero authentication prompts for a local user. Verified with
  `pkcheck`: the four actions are authorized without a challenge; unrelated
  actions (e.g. `set-dnssec`) are still challenged.
- **Least-privilege**: the grant is limited to four DNS actions for local active
  sessions — not a blanket admin bypass. A unit test locks the rule's content so
  it cannot silently widen.
- **Packaging lifecycle**: the rule is installed/removed alongside the binary and
  the `setcap` grant. From-source users get it via `make install` (or copy it
  manually).
- **Scope choice**: we gate on `local && active` rather than a unix group. A
  future refinement could restrict to a dedicated group (e.g. `netdev`) if
  multi-user hardening is desired; out of scope here.
- **No new runtime dependency**: the rule is a static asset; the Rust code is
  unchanged except a clarifying comment and the already-present best-effort
  handling.
