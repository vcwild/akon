# Quickstart: verifying `akon vpn status`

## Automated (offline, CI-compatible)

The status decision is a pure function tested without a live VPN:

```bash
cargo test -p akon  --bin akon status      # CLI decision tests
cargo test -p akon-core netlink            # interface_exists/ipv4 adapter (uses `lo`)
```

Expect: Connected / NotConnected / Stale verdicts for the simulated inputs, and
the PID-independence case (interface present + dead PID ⇒ Connected).

## Manual (with a live connection)

In one terminal:
```bash
akon vpn on
```

In another terminal:
```bash
akon vpn status        # → "active (running)", exit 0, live IP + device
echo $?                # 0
```

After disconnect:
```bash
akon vpn off
akon vpn status        # → "inactive (dead) (not connected)", exit 1
```

## Simulating a stale record (no live VPN)

```bash
export AKON_STATE_FILE=/tmp/akon_status_demo.json
printf '{"backend":"native-f5","device":"tunX-nope","ip":"10.20.30.40","connected_at":"2026-01-01T00:00:00Z","pid":999999}' > "$AKON_STATE_FILE"
akon vpn status        # → "inactive (stale) (tunnel interface no longer present)", exit 2
akon vpn off           # clears the record
akon vpn status        # → "not connected", exit 1
```
