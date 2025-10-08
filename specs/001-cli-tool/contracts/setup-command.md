# CLI Contract: Setup Command

**Command**: `akon setup`
**Priority**: P1 (User Story 1)
**Purpose**: First-time credential configuration

## Signature

```bash
akon setup [OPTIONS]
```

## Options

| Flag | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `--config` | `Path` | No | `~/.config/akon/config.toml` | Override config file location |
| `--server` | `String` | Interactive | N/A | VPN server URL (skips prompt if provided) |
| `--username` | `String` | Interactive | N/A | Username (skips prompt if provided) |
| `--protocol` | `ssl\|nc` | Interactive | `ssl` | VPN protocol |
| `--non-interactive` | Flag | No | false | Fail if stdin not available (CI mode) |

## Behavior

1. Check if GNOME Keyring is accessible
   - If locked: Display error and exit 2
   - If unavailable: Display installation instructions and exit 2

2. Prompt for VPN configuration (if not provided via flags):
   - VPN Server URL (validate format)
   - Username (validate non-empty)
   - Protocol (ssl or nc)
   - Optional: Port number

3. Prompt for OTP secret:
   - Display format hint: "Base32-encoded string (16-52 characters, A-Z, 2-7)"
   - Validate format on input
   - Retry up to 3 times on invalid input
   - Mask input (use `rpassword` crate)

4. Store configuration:
   - Write non-sensitive data to `~/.config/akon/config.toml` (mode 0600)
   - Store OTP secret in GNOME Keyring (service: `akon-vpn-otp`)

5. Verify storage:
   - Attempt to retrieve secret from keyring
   - If retrieval fails: Display error and exit 1

6. Display success message with next steps

## Exit Codes

- `0`: Setup completed successfully
- `1`: Runtime error (keyring operation failed, network issue)
- `2`: Configuration error (invalid input, keyring unavailable)

## Output Examples

### Success

```
🔧 Akon VPN Setup

Enter VPN Server URL: https://vpn.example.com
Enter Username: john.doe
Select Protocol: [ssl] / nc: ssl
Enter OTP Secret (Base32): ****************

✅ Configuration saved to ~/.config/akon/config.toml
✅ OTP secret stored securely in GNOME Keyring

Next steps:
  akon vpn on    - Connect to VPN
  akon get-password - Generate OTP token
```

### Error: Keyring Locked

```
❌ Error: GNOME Keyring is locked

Please unlock your session keyring:
  - Log in to your desktop session
  - Or run: gnome-keyring-daemon --unlock

Exit code: 2
```

### Error: Invalid OTP Secret

```
Enter OTP Secret (Base32): invalid-secret!

❌ Error: Invalid Base32 format
Expected: A-Z, 2-7, optional padding (=)
Example: JBSWY3DPEHPK3PXP

Attempt 1 of 3. Try again:
```

## State Changes

- Creates `~/.config/akon/config.toml` (or overwrites if exists)
- Creates/updates GNOME Keyring entry (service: `akon-vpn-otp`)
- Logs event to systemd journal: `event=setup_complete username=<user>`

## Security Requirements

- OTP secret MUST NOT appear in logs, stdout, or config file
- Config file MUST have permissions 0600 (user-only read/write)
- Keyring entry MUST use encryption (handled by Secret Service)
- Input prompt MUST mask secret (no echo)

## Dependencies

- GNOME Keyring service running
- D-Bus session bus available
- Write access to `~/.config/`

## Test Scenarios

1. **Happy Path**: All inputs valid, keyring accessible → Exit 0
2. **Locked Keyring**: Keyring locked → Display error, exit 2
3. **Invalid OTP Format**: Non-Base32 input → Retry prompt, max 3 attempts
4. **Keyring Unavailable**: Service not running → Display error, exit 2
5. **Non-Interactive Mode**: No stdin + `--non-interactive` → Exit 2
6. **Overwrite Existing**: Config exists → Confirm overwrite, proceed
7. **File Permission Error**: No write access to ~/.config → Display error, exit 2
