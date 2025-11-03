# akon - OTP-Integrated VPN CLI Tool

A secure command-line tool for managing VPN connections with automatic TOTP (Time-based One-Time Password) authentication using GNOME Keyring for secure credential storage.

## Features

- 🔐 **Secure Credential Management**: Stores PIN and TOTP secret securely in GNOME Keyring
- 🚀 **Automatic OTP Generation**: Generates TOTP tokens automatically during connection
- 🔌 **OpenConnect Integration**: Uses OpenConnect CLI for robust VPN connectivity (F5 protocol support)
- ⚡ **Fast & Lightweight**: CLI-based architecture with minimal dependencies
- 📊 **Real-time Progress**: Shows connection progress with detailed status updates
- 🛡️ **Production-Ready**: Comprehensive error handling with actionable suggestions
- 📝 **Excellent Logging**: Systemd journal integration for production debugging

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

# Build release binary
cargo build --release

# Install to /usr/local/bin
sudo cp target/release/akon /usr/local/bin/

# Verify installation
akon --help
```

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
sudo akon vpn on
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
sudo akon vpn off
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

### Keyring Storage

Credentials stored in GNOME Keyring under service name `"akon"`:
- Entry `"pin"`: Your numeric PIN
- Entry `"otp_secret"`: Your TOTP secret (Base32)

## Error Handling

akon provides helpful error messages with actionable suggestions:

### Authentication Failures

```
❌ Error: Authentication failed

💡 Suggestions:
   • Verify your PIN is correct
   • Check if your TOTP secret is valid
   • Run 'akon setup' to reconfigure credentials
   • Ensure your account is not locked
```

### TUN Device Errors

```
❌ Error: Failed to open TUN device - try running with sudo
   Details: failed to open tun device

💡 Suggestions:
   • VPN requires root privileges to create TUN device
   • Run with: sudo akon vpn on
   • Ensure the 'tun' kernel module is loaded
   • Check: lsmod | grep tun
```

### DNS Resolution Errors

```
❌ Error: DNS resolution failed - check server address
   Details: cannot resolve hostname vpn.example.com

💡 Suggestions:
   • Check your DNS configuration
   • Verify the VPN server hostname in config.toml
   • Try using the server's IP address instead
   • Check /etc/resolv.conf for DNS settings
```

### SSL/TLS Errors

```
❌ Error: SSL/TLS connection failure

💡 Suggestions:
   • Check your internet connection
   • Verify the VPN server address is correct
   • The server may be experiencing issues
   • Try again in a few moments
```

## Logging

akon uses structured logging with `tracing`:

### Development

Logs to stderr with pretty formatting:

```bash
RUST_LOG=debug akon vpn on
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

## Troubleshooting

### "OpenConnect not found"

```bash
# Install OpenConnect
sudo apt install openconnect

# Verify
which openconnect
```

### "Permission denied"

Run with sudo:
```bash
sudo akon vpn on
```

### "Failed to access keyring"

Ensure GNOME Keyring is running:
```bash
# Check keyring daemon
ps aux | grep gnome-keyring

# Unlock keyring (if locked)
gnome-keyring-daemon --unlock
```

### Connection Hangs

1. Check OpenConnect directly:
   ```bash
   sudo openconnect --protocol=f5 vpn.example.com
   ```

2. Enable debug logging:
   ```bash
   RUST_LOG=debug sudo akon vpn on
   ```

3. Check system logs:
   ```bash
   sudo journalctl -xe
   ```

### Stale State

If status shows "Stale state":
```bash
sudo akon vpn off  # Cleanup
```

## Development

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run specific test suite
cargo test -p akon-core output_parser

# Check code
cargo clippy --all-targets
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

**Current Coverage**: 139 tests, all passing ✅

### Adding New Error Patterns

To add new OpenConnect error patterns:

1. Add regex pattern to `OutputParser::new()` in `akon-core/src/vpn/output_parser.rs`
2. Add pattern matching in `OutputParser::parse_error()`
3. Add error variant to `VpnError` in `akon-core/src/error.rs` if needed
4. Add suggestion handler to `print_error_suggestions()` in `src/cli/vpn.rs`
5. Write tests in `akon-core/tests/output_parser_tests.rs`

Example:
```rust
// In OutputParser::new()
let new_error_pattern = Regex::new(r"(?i)some error pattern").unwrap();

// In parse_error()
if self.new_error_pattern.is_match(line) {
    return ConnectionEvent::Error {
        kind: VpnError::SomeNewError { /* fields */ },
        raw_output: line.to_string(),
    };
}
```

## Security Considerations

- **Credentials**: Never logged, stored only in encrypted keyring
- **Password**: Passed via stdin (not command-line arguments)
- **State File**: Contains IP/PID only, no secrets (`/tmp/akon_vpn_state.json`)
- **Process Cleanup**: Ensures OpenConnect terminates on exit
- **Safe Code**: Zero unsafe blocks in VPN modules

## Performance

- **Connection Time**: < 30 seconds typical
- **Parse Latency**: < 500ms per output line
- **Memory Usage**: ~10MB resident
- **CPU Usage**: Minimal (event-driven architecture)

## Contributing

Contributions welcome! Please:

1. Follow existing code style
2. Add tests for new features
3. Update documentation
4. Run `cargo clippy` before submitting
5. Ensure all tests pass: `cargo test`

## License

[Add your license here]

## Author

Victor Wild (vcwild)

## Acknowledgments

- OpenConnect project for VPN functionality
- GNOME Keyring for secure credential storage
- Tokio async runtime
- Rust community

## Support

- Issues: https://github.com/vcwild/akon/issues
- Discussions: https://github.com/vcwild/akon/discussions

---

**Note**: This tool is designed for F5 SSL VPN protocol. Other protocols may work but are not officially supported.
