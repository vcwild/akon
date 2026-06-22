# Contract: `akon vpn on` background mode

## CLI interface

```
akon vpn on [--foreground]
```

- Default: backgrounds after `Connected` (returns prompt to user, exit 0).
- `--foreground` / `-f`: blocks until the session ends (existing behaviour).

## Exit codes (foreground parent process)

| Code | Meaning |
|------|---------|
| `0` | Connected successfully; VPN is running in background |
| `1` | Connection failed; error printed to terminal |

## Standard output (foreground, default mode)

```
>> Connecting to VPN server (native F5): <server>
[AUTH] Authenticating...
[OK] VPN connection established
   IP address: <assigned-ip>
   Running in background (logs: ~/.local/share/akon/vpn.log)
   Run 'akon vpn off' to disconnect.
```

After this output the shell prompt returns.

## Background process

- New session (`setsid`), no controlling terminal.
- `stdin` → `/dev/null`.
- `stdout`/`stderr` → `~/.local/share/akon/vpn.log` (appended).
- Writes the state file before signalling the parent (so `vpn status`/`vpn off`
  work immediately after `vpn on` returns).
- Continues running the full native VPN (health checks, reconnection) identically
  to the former foreground process.

## ConnectResult pipe protocol

4 bytes: `0x00` + 3 reserved = success; first byte `0x01` = failure.
Then a length-prefixed UTF-8 string: on success `"<ip>\n<device>"`, on failure
the error message. Parent reads up to 512 bytes within a 30s timeout.

## Log file location

`$XDG_DATA_HOME/akon/vpn.log` or `~/.local/share/akon/vpn.log`.
Created (with parent dirs) by the background child on first run.
