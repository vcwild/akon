# Akon VPN CLI - Quick Start Guide

**Version**: 0.1.0 (MVP - Setup, Connect, Disconnect)
**Date**: 2025-10-08

## Overview

Akon is a secure CLI tool for connecting to VPN servers with OTP (One-Time Password) authentication. It automates TOTP token generation and manages VPN lifecycle through the OpenConnect protocol.

## Prerequisites

### System Requirements

- **OS**: Linux (GNOME desktop environment)
- **Required Packages**:
  - GNOME Keyring (`gnome-keyring`)
  - OpenConnect library (`libopenconnect`)
  - Rust toolchain 1.70+ (for building from source)

### Installation

```bash
# Install system dependencies (Debian/Ubuntu)
sudo apt-get install gnome-keyring openconnect libsecret-1-dev pkg-config

# Install system dependencies (Fedora/RHEL)
sudo dnf install gnome-keyring openconnect libsecret-devel

# Clone repository
git clone https://github.com/your-org/akon.git
cd akon

# Build and install
cargo build --release
sudo cp target/release/akon /usr/local/bin/
```

## Quick Start

### Step 1: Initial Setup

Configure your VPN credentials and OTP secret:

```bash
akon setup
```

You'll be prompted for:

- **VPN Server URL**: `https://vpn.example.com`
- **Username**: `john.doe`
- **Protocol**: `ssl` or `nc` (AnyConnect)
- **OTP Secret**: Your Base32-encoded TOTP seed (from your organization's 2FA setup)

**Important**: The OTP secret is stored securely in GNOME Keyring, NOT in plain text files.

### Step 2: Connect to VPN

```bash
akon vpn on
```

This will:

1. Generate a fresh OTP token from your secret
2. Authenticate with the VPN server
3. Establish the VPN connection
4. Run as a background daemon

Expected output:

```
🔄 Connecting to vpn.example.com...
🔑 Generating OTP token...
🔐 Authenticating...
✅ VPN connected successfully!

Server: vpn.example.com (SSL)
Interface: tun0
IP Address: 10.0.1.42
```

### Step 3: Disconnect from VPN

```bash
akon vpn off
```

This gracefully tears down the connection and cleans up the daemon process.

## Common Commands

### Check Connection Status

```bash
akon vpn status
```

Shows current connection state, uptime, and server details.

### Generate OTP Token (Manual Use)

```bash
akon get-password
```

Outputs only the current TOTP token to stdout (useful for piping or manual browser login):

```bash
# Copy token to clipboard
akon get-password | pbcopy

# Use in a script
TOKEN=$(akon get-password)
echo "Your OTP token: $TOKEN"
```

### Override Config File Location

```bash
akon --config /path/to/custom/config.toml vpn on
```

## Configuration

### Config File Location

Default: `~/.config/akon/config.toml`

### Config File Format

```toml
server = "https://vpn.example.com"
username = "john.doe"
protocol = "ssl"
# port = 443  # Optional: override default port
```

**Security Note**: This file contains NO secrets. The OTP secret is stored separately in GNOME Keyring.

### Keyring Storage

- **Service Name**: `akon-vpn-otp`
- **Attributes**: `{ application: "akon", type: "otp", username: "<your-username>" }`

View stored secrets using GNOME Seahorse or `secret-tool`:

```bash
# List akon secrets
secret-tool search application akon

# Retrieve OTP secret (for debugging)
secret-tool lookup application akon type otp username john.doe
```

## Troubleshooting

### Keyring is Locked

**Error**: "GNOME Keyring is locked"

**Solution**: Unlock your session keyring:

```bash
# Ensure keyring daemon is running
gnome-keyring-daemon --start

# Or log in to your desktop session (keyring unlocks automatically)
```

### Authentication Failed

**Error**: "Authentication failed"

**Possible Causes**:

1. **Incorrect OTP Secret**: Run `akon setup` again to reconfigure
2. **Clock Skew**: Check system time synchronization
   ```bash
   timedatectl status
   sudo systemctl restart systemd-timesyncd
   ```
3. **Server Issue**: Verify server URL and credentials manually

### Network Timeout

**Error**: "Network error: Connection timed out"

**Possible Causes**:

1. **Firewall Blocking VPN**: Check firewall rules
2. **Server Unreachable**: Verify server is online
3. **Network Interface Down**: Check network connectivity
   ```bash
   ip link show
   ping vpn.example.com
   ```

### Daemon Won't Stop

**Error**: "Daemon did not respond to shutdown request"

**Solution**: Force kill the daemon

```bash
akon vpn off --force
```

### View Logs

All events are logged to systemd journal:

```bash
# Last 50 lines
journalctl -t akon -n 50

# Follow logs in real-time
journalctl -t akon -f

# Filter by event type
journalctl -t akon | grep event=connection_attempt
```

## Security Best Practices

1. **Protect Config Directory**:
   ```bash
   chmod 700 ~/.config/akon
   chmod 600 ~/.config/akon/config.toml
   ```

2. **Verify Keyring Access**:
   - Only your user should have access to GNOME Keyring
   - Use full-disk encryption for additional protection

3. **Audit Logs Regularly**:
   ```bash
   journalctl -t akon | grep event=keyring_access
   ```

4. **Rotate OTP Secret Periodically**:
   - Coordinate with your IT team
   - Run `akon setup` to update

## Advanced Usage

### Run in Foreground (for Debugging)

```bash
akon vpn on --foreground
```

Press Ctrl+C to disconnect.

### Custom Connection Timeout

```bash
akon vpn on --timeout 60  # Wait up to 60 seconds
```

### Non-Interactive Mode (CI/CD)

```bash
akon setup --non-interactive \
  --server https://vpn.example.com \
  --username ci-bot \
  --protocol ssl
```

(OTP secret must be provided via stdin or environment variable in future versions)

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        Akon CLI                             │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────┐    │
│  │  Setup   │    │ VPN On   │    │    VPN Off       │    │
│  │ Command  │    │ Command  │    │    Command       │    │
│  └────┬─────┘    └────┬─────┘    └────┬─────────────┘    │
│       │               │               │                    │
│       │               │               │                    │
│  ┌────▼───────────────▼───────────────▼─────────────┐    │
│  │         akon-core Library                        │    │
│  │  ┌──────────┐ ┌──────────┐ ┌─────────────────┐  │    │
│  │  │  Auth    │ │  Config  │ │  VPN Connection │  │    │
│  │  │ (Keyring │ │  (TOML)  │ │  (OpenConnect   │  │    │
│  │  │  + OTP)  │ │          │ │   FFI)          │  │    │
│  │  └────┬─────┘ └────┬─────┘ └────┬────────────┘  │    │
│  └───────┼────────────┼────────────┼───────────────┘    │
│          │            │            │                      │
└──────────┼────────────┼────────────┼──────────────────────┘
           │            │            │
           ▼            ▼            ▼
    ┌────────────┐ ┌────────┐ ┌──────────────┐
    │   GNOME    │ │  TOML  │ │ libopenconnect│
    │  Keyring   │ │  File  │ │  (C library)  │
    │ (D-Bus API)│ │        │ │               │
    └────────────┘ └────────┘ └───────────────┘
```

## What's Next

### Upcoming Features (Not in this Release)

- **Status Command**: `akon vpn status` - Check connection details
- **Get Password**: `akon get-password` - Manual OTP generation
- **Auto-Reconnect**: Background monitoring service (User Story 5)
- **Config Management**: Update config without re-running setup
- **Multiple Profiles**: Switch between different VPN servers

### Contributing

See `CONTRIBUTING.md` for development guidelines.

### Support

- **Issues**: https://github.com/your-org/akon/issues
- **Docs**: https://akon.example.com/docs
- **Chat**: #akon on Slack/Discord

## Exit Codes Reference

| Code | Meaning | Examples |
|------|---------|----------|
| `0` | Success | Command completed successfully |
| `1` | Runtime Error | Authentication failed, network timeout, keyring operation failed |
| `2` | Configuration Error | Missing setup, invalid config, keyring unavailable |

## License

MIT License - See `LICENSE` file for details.

---

**Need Help?** Check logs first: `journalctl -t akon -n 50`
