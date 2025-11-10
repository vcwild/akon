  <div align="center">
    <img src=".github/assets/badge.svg" width="200px" alt="akon badge" />
  </div>
  <div align="center">
    <img src="https://img.shields.io/github/v/release/vcwild/akon?include_prereleases&color=%23FFCE69&style=flat-square" alt="release" />
    <img src="https://img.shields.io/github/license/vcwild/akon?color=%23FFCE69&style=flat-square" alt="license" />
    <img src="https://img.shields.io/github/repo-size/vcwild/akon?color=%23FFCE69&style=flat-square" alt="repo size" />
  </div>

# akon - OTP-Integrated VPN Tool

A secure command-line tool for managing VPN connections with automatic TOTP (Time-based One-Time Password) authentication using GNOME Keyring for secure credential storage.

## Features

- **Secure Credential Management**: Stores PIN and TOTP secret securely in GNOME Keyring
- **Automatic OTP Generation**: Generates TOTP tokens automatically during connection
- **OpenConnect Integration**: Uses OpenConnect CLI for robust VPN connectivity (F5 protocol support)
- **Automatic Reconnection**: Detects network interruptions and reconnects with exponential backoff
- **Health Monitoring**: Periodic health checks detect silent VPN failures
- **Fast & Lightweight**: CLI-based architecture with minimal dependencies

## Architecture

akon uses a **CLI process delegation** architecture:
- Spawns OpenConnect as a child process
- Manages process lifecycle (spawn → monitor → terminate)
- Parses output in real-time for connection events
- Provides clean async API using Tokio

This design eliminates FFI complexity while maintaining full OpenConnect functionality.

## Requirements

- **Operating System**: Linux (tested on Ubuntu/Debian, RHEL/Fedora)
- **OpenConnect**: Version 9.x or later
  ```bash
  # Ubuntu/Debian
  sudo apt install openconnect

  # RHEL/Fedora
  sudo dnf install openconnect

  # Verify installation
  which openconnect
  ```
- **GNOME Keyring**: For secure credential storage
  ```bash
  sudo apt install gnome-keyring libsecret-1-dev
  ```
- **Root Privileges**: Required for TUN device creation (run with `sudo`)

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/vcwild/akon.git
cd akon

# Build and install (sets up passwordless sudo for openconnect)
make install

# Verify installation
akon --help
```

**What `make install` does:**
- Builds the release binary
- Installs to `/usr/local/bin/akon`
- Configures passwordless sudo for openconnect
- No password prompts when connecting to VPN!

## Quick Start

### 1. Setup Credentials

Store your VPN credentials securely:

```bash
akon setup
```

You'll be prompted for:
- **Server**: VPN server hostname (e.g., `vpn.example.com`)
- **Username**: Your VPN username
- **PIN**: Your numeric PIN
- **TOTP Secret**: Your TOTP secret key (Base32 encoded)

These credentials are stored in:
- Config file: `~/.config/akon/config.toml` (server, username, protocol)
- Keyring: GNOME Keyring (PIN and TOTP secret - encrypted)

### 2. Connect to VPN

```bash
akon vpn on
```

**Why sudo?** OpenConnect needs root privileges to create the TUN network device.

**What happens:**
1. Loads config from `~/.config/akon/config.toml`
2. Retrieves PIN and TOTP secret from keyring
3. Generates current TOTP token
4. Spawns OpenConnect with credentials
5. Monitors connection progress
6. Reports IP address when connected

### 3. Check Status

```bash
akon vpn status
```

**Outputs:**
- **Connected** (exit code 0): Shows IP, device, duration, PID
- **Not connected** (exit code 1): No active connection
- **Stale state** (exit code 2): Process died, cleanup needed

### 4. Disconnect

```bash
akon vpn off
```

**Disconnect flow:**
1. Sends SIGTERM for graceful shutdown (5s timeout)
2. Falls back to SIGKILL if process doesn't respond
3. Cleans up state file

### 5. Manual OTP Generation

Generate OTP token for manual use:

```bash
akon get-password
```

Outputs PIN+TOTP combined password (does not initiate connection).

## Configuration

### Config File Location

`~/.config/akon/config.toml`

### Example Configuration

```toml
[vpn]
server = "vpn.example.com"
username = "your.username"
protocol = "f5"  # F5 SSL VPN protocol

# Optional settings
timeout = 60
no_dtls = false
lazy_mode = true  # Connect VPN when running 'akon' without arguments
```

### Lazy Mode

When `lazy_mode = true` is set in your configuration, running `akon` without any arguments will automatically connect to the VPN:

```bash
# With lazy mode enabled, these are equivalent:
akon
akon vpn on

# With lazy mode disabled, akon without args shows help
akon  # Shows usage information
```

This feature is perfect for quick VPN connections - just type `akon` and go!

### Automatic Reconnection

akon automatically detects network interruptions and reconnects your VPN with intelligent retry logic.

#### Configuration

Add a `[reconnection]` section to your config to enable automatic reconnection:

```toml
[vpn]
server = "vpn.example.com"
username = "your.username"
protocol = "f5"

[reconnection]
# Required: HTTP/HTTPS endpoint to check connectivity
health_check_endpoint = "https://your-internal-server.example.com/"

# Optional: Customize retry behavior (defaults shown)
max_attempts = 5              # Maximum reconnection attempts
base_interval_secs = 5        # Initial retry delay
backoff_multiplier = 2        # Exponential backoff multiplier
max_interval_secs = 60        # Maximum delay between attempts
consecutive_failures_threshold = 2  # Health check failures before reconnection
health_check_interval_secs = 60     # How often to check health
```

#### How It Works

**Network Interruption Detection:**
- Monitors NetworkManager via D-Bus
- Detects WiFi changes, suspend/resume, interface changes
- Automatically triggers reconnection when network returns

**Health Monitoring:**
- Periodic HTTP checks to configured endpoint
- Detects silent VPN failures (connection alive but traffic not flowing)
- Triggers reconnection after consecutive failure threshold

**Exponential Backoff:**
- First attempt: 5 seconds
- Second attempt: 10 seconds
- Third attempt: 20 seconds
- Fourth attempt: 40 seconds
- Fifth attempt: 60 seconds (capped)

**Example Reconnection Flow:**
```text
1. VPN Connected → Network interruption detected
2. Attempt 1 (after 5s) → Failed
3. Attempt 2 (after 10s) → Failed
4. Attempt 3 (after 20s) → Success!
5. VPN Connected → Continues monitoring
```

#### Manual Recovery Commands

If automatic reconnection fails after max attempts:

```bash
# 1. Check status (shows error and manual steps)
akon vpn status

# Output:
# ● Status: Error - Max reconnection attempts exceeded
#   Last error: Connection refused after 5 attempts
#   ❌ Failed after 5 reconnection attempts
#
# ⚠ Manual intervention required:
#   1. Run akon vpn cleanup to terminate orphaned processes
#   2. Run akon vpn reset to clear retry counter
#   3. Run akon vpn on to reconnect

# 2. Cleanup orphaned processes
sudo akon vpn cleanup

# 3. Reset retry counters
akon vpn reset

# 4. Try connecting again
sudo akon vpn on
```


### Production (systemd)

Automatically detects systemd and logs to journal:

```bash
# View logs
journalctl -f -u akon

# View with priority filter
journalctl -f -u akon -p info
```

### Log Levels

- `ERROR`: Connection failures, critical errors
- `WARN`: Force-kill fallback, degraded operations
- `INFO`: State transitions, successful operations (default)
- `DEBUG`: OpenConnect output parsing, detailed flow
- `TRACE`: Very verbose debugging (not used)

### Example Log Output

```
INFO akon::cli::vpn: Loaded configuration for server: vpn.example.com
INFO akon::cli::vpn: Generated VPN password from keyring credentials
INFO akon::cli::vpn: Created CLI connector
INFO akon_core::vpn::cli_connector: OpenConnect process spawned with PID: 12345
INFO akon::cli::vpn: Authentication in progress phase="authentication" message="Connecting to server"
INFO akon::cli::vpn: F5 session established phase="session"
INFO akon::cli::vpn: TUN device configured device="tun0" ip="10.0.1.100"
INFO akon::cli::vpn: VPN connection fully established ip="10.0.1.100" device="tun0"
```

### Project Structure

```
akon/
├── akon-core/          # Core library
│   ├── src/
│   │   ├── auth/       # OTP, keyring, password generation
│   │   ├── config/     # TOML configuration
│   │   ├── vpn/        # VPN connection management
│   │   │   ├── cli_connector.rs    # OpenConnect process manager
│   │   │   ├── output_parser.rs    # Output parsing with regex
│   │   │   └── connection_event.rs # Event types
│   │   └── error.rs    # Error types
│   └── tests/          # Unit tests
├── src/                # CLI application
│   ├── cli/            # Command implementations
│   │   ├── setup.rs    # Setup command
│   │   └── vpn.rs      # VPN commands (on/off/status)
│   └── main.rs         # Entry point
└── tests/              # Integration tests
```

### Test Coverage

```bash
# Run all tests
cargo test

# Run with coverage (requires cargo-tarpaulin)
cargo install cargo-tarpaulin
cargo tarpaulin --out Html

# View coverage report
open tarpaulin-report.html
```

## Contributing

Contributions are welcome! Please:

1. Follow existing code style
2. Add tests for new features
3. Update documentation
4. Run `cargo clippy` before submitting
5. Ensure all tests pass: `cargo test`

## License

This project is licensed under the MIT license.

## Support

- Issues: https://github.com/vcwild/akon/issues
- Discussions: https://github.com/vcwild/akon/discussions

---

**Note**: This tool is designed for F5 SSL VPN protocol. Other protocols may work but are not officially supported.
