# Data Model: `akon vpn status`

## Session record (persisted; written by `akon vpn on`)

The on-disk snapshot of the connection state machine. Status reads it as metadata;
the tunnel interface is authoritative for "connected".

| Field | Type | Source | Used by status for |
|-------|------|--------|--------------------|
| `backend` | string (`"native-f5"`) | connect | (informational) |
| `server` | string | config | (informational) |
| `device` | string (e.g. `tun0`) | connect | **ground-truth lookup** (does this interface exist?) |
| `ip` | string (IPv4) | connect | fallback IP if live read fails |
| `connected_at` | RFC3339 string | connect | uptime / "active since" |
| `pid` | number | connect | **advisory** owner PID (may note "not running") |
| `teardown_plan` | object | connect | used by `vpn off` (not status) |

## ConnectionState (in-memory state machine — `akon-core/src/vpn/state.rs`)

`Disconnected → Connecting → Connected(metadata) → Disconnecting`
(with `Error(..)` and `Reconnecting{..}` branches). The persisted record is a
snapshot of the `Connected` state's metadata.

## StatusVerdict (status decision output)

Pure result of reconciling the record against ground truth.

| Verdict | When | Exit code |
|---------|------|-----------|
| `Connected { device, ip, since }` | record present AND `device` interface exists | 0 |
| `Stale { reason }` | record present BUT `device` interface absent (or no `device` recorded) | 2 |
| `NotConnected` | no record | 1 |

## Decision inputs (the pure function)

`evaluate_status(record: Option<Record>, interface_present: bool, live_ip: Option<String>) -> StatusVerdict`

- `record == None` → `NotConnected`
- `record.device` absent → `Stale { "no tunnel device recorded" }`
- `interface_present == false` → `Stale { "tunnel interface no longer present" }`
- else → `Connected { device, ip: live_ip.or(record.ip), since: record.connected_at }`

The PID is **not** an input to the verdict (FR-005); it is only used for the
advisory "(not running)" annotation in the Connected output.
