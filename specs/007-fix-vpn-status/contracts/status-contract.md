# Contract: `akon vpn status`

`status` is read-only and privilege-free. It never modifies host state and never
prompts for sudo.

## Exit codes (STABLE — for scripting)

| Code | Meaning |
|------|---------|
| `0` | Connected — a tunnel interface for the recorded session exists |
| `1` | Not connected — no session recorded |
| `2` | Stale — a session is recorded but its tunnel interface is gone |

These match the pre-existing exit codes (1 = not connected, 2 = stale) and add a
clear 0 for connected.

## Output (human-readable)

### Connected (exit 0)
```
● akon-vpn - Akon VPN Connection
    Active: active (running) since <local time>; <uptime> ago
  Main PID: <pid> (akon native F5)[ (not running)]
        IP: <live ipv4>
    Device: <tunX>
```
- `IP` is read **live** from the interface; falls back to the recorded IP.
- `(not running)` is appended to the PID line only when the recorded owner PID is
  not alive (the tunnel still exists, so the verdict stays Connected).

### Not connected (exit 1)
```
● akon-vpn - Akon VPN Connection
    Active: inactive (dead) (not connected)
```

### Stale (exit 2)
```
● akon-vpn - Akon VPN Connection
    Active: inactive (stale) (<reason>)
   Last IP: <recorded ipv4>

  [TIP] Run akon vpn off to clean up stale state
```
- `<reason>` is "tunnel interface no longer present" or "no tunnel device recorded".

## Robustness

- Missing state file → exit 1 (not an error).
- Corrupt/unreadable state file → clear error message, non-zero exit, no panic.
- Non-Linux → interface lookup returns "not present"; behaves as Stale/NotConnected
  without crashing.
