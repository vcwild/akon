# Quickstart: verifying no DNS password prompts

## The fix

akon ships a polkit rule (`packaging/polkit/49-akon-resolved-dns.rules`) installed
to `/usr/share/polkit-1/rules.d/`. It lets a local, active user apply/revert the
VPN tunnel's DNS via systemd-resolved without an authentication prompt.

## Installed automatically

- **deb/rpm packages**: the rule is a packaged asset (installed on install,
  removed on uninstall).
- **`make install`**: copies the rule to the polkit rules dir (needs root, like
  the `setcap` grant). `make uninstall` removes it.

## Manual install (from source, without `make install`)

```bash
sudo install -d -m 755 /usr/share/polkit-1/rules.d
sudo install -m 644 packaging/polkit/49-akon-resolved-dns.rules \
    /usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules
```

Modern polkit (≥ 0.106) reloads `.rules` files automatically; no reboot needed.

## Verify (no live VPN required)

`pkcheck` asks polkit whether the current session is authorized for an action.
With the rule installed, the four resolve1 DNS actions must be authorized
**without a prompt** (exit 0), and unrelated actions must still be challenged:

```bash
# Granted (expect exit 0 = authorized, no prompt):
for a in set-dns-servers set-domains set-default-route revert; do
  pkcheck --action-id org.freedesktop.resolve1.$a --process $$ \
    && echo "$a: AUTHORIZED" || echo "$a: challenge"
done

# Not granted (expect a challenge — proves the rule is scoped):
pkcheck --action-id org.freedesktop.resolve1.set-dnssec --process $$ \
  && echo "unexpected" || echo "set-dnssec: challenge (correct)"
```

## End-to-end (with a live VPN)

```bash
akon vpn on        # connects with ZERO password prompts; VPN DNS applied
akon vpn off       # disconnects + reverts DNS, also without a prompt
```

## Without the rule (graceful degradation)

If the rule is absent (or there is no polkit/resolved), `akon vpn on` still
brings up the tunnel; DNS application is best-effort and prints a warning
("failed to apply VPN DNS — names may not resolve"). It never hangs on a prompt
in a non-interactive context.
