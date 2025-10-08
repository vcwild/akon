# Phase 0: Research & Technology Decisions

**Feature**: OTP-Integrated VPN CLI with Secure Credential Management
**Date**: 2025-10-08
**Status**: Complete

## Overview

This document consolidates research findings for technology choices and implementation patterns for the Akon VPN CLI tool.

## 1. Rust FFI to libopenconnect

### Decision

Use `bindgen` to generate Rust FFI bindings to the libopenconnect C library in a `build.rs` script. Wrap unsafe FFI calls in safe Rust abstractions within the `vpn::openconnect` module.

### Rationale

- **Direct Control**: In-process FFI gives precise control over connection lifecycle, callbacks, and error handling
- **Performance**: No process spawning overhead, minimal serialization cost
- **Safety**: Rust's ownership model prevents common C interop bugs (use-after-free, null pointer dereference)
- **Community Practice**: Standard Rust pattern for C library integration (e.g., `openssl-sys`, `libsqlite3-sys`)

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Spawn `openconnect` CLI | Cannot capture real-time connection events, password passing via stdin is less secure than in-memory callbacks, harder to distinguish error categories |
| Write pure Rust OpenConnect client | Massive scope increase (thousands of lines), VPN protocol complexity, SSL/TLS handling, would take months |
| Use existing `openconnect` Rust crate | No mature crate exists (as of 2025-10-08); would need to create it anyway |

### Implementation Notes

```rust
// build.rs
fn main() {
    println!("cargo:rustc-link-lib=openconnect");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")  // #include <openconnect.h>
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
```

**Key FFI Safety Patterns**:

- Wrap raw pointers in Rust structs with Drop impl
- Use `CString` for C string conversion
- Document unsafe blocks with invariants
- Never expose `unsafe` in public API

**References**:

- OpenConnect C API: <https://www.infradead.org/openconnect/api.html>
- Rust FFI Guide: <https://doc.rust-lang.org/nomicon/ffi.html>
- bindgen Tutorial: <https://rust-lang.github.io/rust-bindgen/>

---

## 2. GNOME Keyring Integration

### Decision

Use the `secret-service` Rust crate to interact with GNOME Keyring via the Secret Service D-Bus API.

### Rationale

- **Pure Rust**: No need for C bindings to libsecret
- **Standard Protocol**: Secret Service API is FreeDesktop.org standard, works with GNOME Keyring, KWallet, and other backends
- **Async-Free**: `secret-service` supports blocking API, aligns with project constraint (no async runtime)
- **Well-Maintained**: Active maintenance, good documentation

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| `libsecret` crate (FFI) | Adds another FFI dependency, more unsafe code, less idiomatic Rust |
| `keyring` crate | High-level abstraction but depends on platform detection; Secret Service API is more explicit for Linux-first approach |
| Direct D-Bus calls | Reinvents the wheel, Secret Service protocol is complex (encryption, session negotiation) |

### Implementation Notes

```rust
use secret_service::{EncryptionType, SecretService};
use secrecy::{Secret, ExposeSecret};

pub fn store_otp_secret(secret: Secret<String>) -> Result<()> {
    let ss = SecretService::connect(EncryptionType::Dh)?;
    let collection = ss.get_default_collection()?;

    collection.create_item(
        "Akon VPN OTP Secret",
        vec![("application", "akon"), ("type", "otp")],
        secret.expose_secret().as_bytes(),
        true,  // replace existing
        "text/plain"
    )?;

    Ok(())
}
```

**Key Patterns**:

- Check for locked keyring: `collection.is_locked()` → prompt user
- Search by attributes: `collection.search_items(vec![("application", "akon")])`
- Delete on setup re-run: `item.delete()`

**References**:

- `secret-service` docs: <https://docs.rs/secret-service/>
- Secret Service spec: <https://specifications.freedesktop.org/secret-service/>

---

## 3. TOTP Implementation

### Decision

Use the `totp-lite` crate for TOTP token generation (RFC 6238).

### Rationale

- **Minimal Dependencies**: Small, focused library (no bloat)
- **RFC Compliant**: Implements RFC 6238 correctly (HMAC-SHA1/SHA256/SHA512)
- **No Async**: Synchronous API fits project constraints
- **Easy Base32 Handling**: Accepts Base32-encoded secrets directly

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| `oath` crate | Requires OpenSSL, heavier dependency chain |
| `google-authenticator` crate | Abandoned (last update 2018) |
| Custom TOTP impl | Security-critical code should use battle-tested libraries; HMAC edge cases are subtle |

### Implementation Notes

```rust
use totp_lite::{totp, Sha1};
use secrecy::{Secret, ExposeSecret};

pub fn generate_totp(secret_base32: &Secret<String>) -> Result<String> {
    let secret_bytes = base32::decode(
        base32::Alphabet::RFC4648 { padding: false },
        secret_base32.expose_secret()
    )?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let token = totp::<Sha1>(&secret_bytes, 30, now);  // 30-second time step
    Ok(format!("{:06}", token))  // Zero-pad to 6 digits
}
```

**Key Patterns**:

- Validate Base32 input during setup (alphabet check)
- Handle clock skew: warn if NTP not synchronized
- Support both 6 and 8 digit codes (server-dependent)

**References**:

- RFC 6238 (TOTP): <https://datatracker.ietf.org/doc/html/rfc6238>
- `totp-lite` docs: <https://docs.rs/totp-lite/>

---

## 4. Error Handling Strategy

### Decision

Use `thiserror` for library error types (structured variants) and `anyhow` for application-level error context.

### Rationale

- **Idiomatic**: This is the standard Rust error handling pattern (used by `tokio`, `serde`, etc.)
- **Type Safety**: `thiserror` generates `std::error::Error` impls, enabling `?` operator
- **Context**: `anyhow::Context` adds user-friendly error messages without polluting error types
- **Exit Codes**: Easy to map error variants to exit codes (0/1/2)

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Custom error enum only | Verbose boilerplate (Display, Error, From impls), no context chaining |
| `eyre` / `color-eyre` | Adds ANSI color rendering, overkill for CLI; `anyhow` is lighter |
| `failure` crate | Deprecated, superseded by `thiserror` + `anyhow` |

### Implementation Notes

```rust
// akon-core/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KeyringError {
    #[error("GNOME Keyring is locked. Please unlock your session keyring.")]
    Locked,

    #[error("GNOME Keyring service not available. Install gnome-keyring package.")]
    ServiceUnavailable,

    #[error("Failed to store secret: {0}")]
    StoreFailed(String),
}

#[derive(Error, Debug)]
pub enum VpnError {
    #[error("VPN authentication failed. Check credentials and OTP secret.")]
    AuthenticationFailed,

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("OpenConnect library error: {0}")]
    LibraryError(String),
}

// CLI code
use anyhow::{Context, Result};

fn setup_command() -> Result<()> {
    let secret = read_otp_secret()
        .context("Failed to read OTP secret from prompt")?;

    keyring::store_otp_secret(secret)
        .context("Failed to store OTP secret in GNOME Keyring")?;

    Ok(())
}
```

**Error-to-Exit-Code Mapping**:

- `Ok(())` → exit 0
- `KeyringError::*`, `ConfigError::*` → exit 2 (configuration error)
- `VpnError::AuthenticationFailed`, `VpnError::NetworkError` → exit 1 (runtime error)

**References**:

- `thiserror` docs: <https://docs.rs/thiserror/>
- `anyhow` docs: <https://docs.rs/anyhow/>

---

## 5. CLI Framework

### Decision

Use `clap` v4 with derive macros for argument parsing.

### Rationale

- **Industry Standard**: Most popular Rust CLI framework (50M+ downloads)
- **Declarative**: Derive macros reduce boilerplate
- **Rich Features**: Subcommands, validation, help generation, shell completion
- **Type Safety**: Compile-time guarantees for argument types

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| `structopt` | Deprecated, merged into clap v3+ |
| `argh` | Simpler but lacks advanced features (env vars, value hints) |
| Manual parsing | Error-prone, no help generation, reinvents the wheel |

### Implementation Notes

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "akon")]
#[command(about = "OTP-integrated VPN CLI with secure credential management")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Override config file location
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// First-time setup: store VPN credentials
    Setup,

    /// VPN connection management
    Vpn {
        #[command(subcommand)]
        action: VpnAction,
    },

    /// Generate OTP token for external use
    GetPassword,
}

#[derive(Subcommand)]
enum VpnAction {
    /// Connect to VPN (spawn background daemon)
    On,

    /// Disconnect from VPN
    Off,

    /// Check VPN connection status
    Status,
}
```

**Key Patterns**:

- Use `#[arg(env)]` for environment variable overrides
- Implement `--quiet` / `--verbose` for logging control
- Generate shell completions in build script

**References**:

- clap docs: <https://docs.rs/clap/>
- clap derive tutorial: <https://docs.rs/clap/latest/clap/_derive/index.html>

---

## 6. Logging Strategy

### Decision

Use `tracing` for structured logging with `tracing-journald` backend for systemd journal integration.

### Rationale

- **Structured**: Key-value pairs enable filtering (e.g., `journalctl -t akon event=connection_attempt`)
- **Context**: Span-based context (e.g., all logs within a connection attempt share request_id)
- **Performance**: Compile-time filtering reduces runtime overhead
- **Ecosystem**: De facto standard for modern Rust (replaces `log` crate)

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| `log` crate | Unstructured text logs, no span context, manual formatting |
| `env_logger` | Development-only, not suitable for production systemd integration |
| `slog` | More complex API, `tracing` has better ecosystem momentum |

### Implementation Notes

```rust
use tracing::{info, warn, error, instrument};
use tracing_subscriber::layer::SubscriberExt;

pub fn init_logging() {
    let journald = tracing_journald::layer()
        .expect("Failed to connect to journald");

    tracing_subscriber::registry()
        .with(journald)
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

#[instrument(skip(secret))]  // Don't log secret parameter
pub fn generate_totp(secret: &Secret<String>) -> Result<String> {
    info!("Generating TOTP token");
    // ... implementation
    info!("TOTP generation successful");
    Ok(token)
}
```

**Security Patterns**:

- Use `#[instrument(skip(...))]` for sensitive parameters
- Never log `Secret<T>` values (Debug impl is redacted)
- Use custom Debug impls for types containing secrets

**References**:

- tracing docs: <https://docs.rs/tracing/>
- tracing-journald: <https://docs.rs/tracing-journald/>

---

## 7. Background Daemon Process Model

### Decision

Use `daemonize` crate to fork background process on `vpn on`, with Unix domain socket for IPC.

### Rationale

- **Standard Unix Pattern**: Fork + setsid + chdir is established daemon practice
- **Terminal Control**: Parent blocks until child signals connection success/failure, then exits
- **Robustness**: Daemon survives parent exit, independent lifecycle
- **IPC**: Unix socket enables bidirectional communication (status queries, shutdown commands)

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| systemd user service | Requires systemd unit file installation, complicates single-binary distribution |
| Foreground process with Ctrl+Z | User must manually background, error-prone, not idempotent |
| `tokio` background task | Requires async runtime, violates project constraint |

### Implementation Notes

```rust
use daemonize::Daemonize;
use std::os::unix::net::{UnixListener, UnixStream};

pub fn spawn_daemon() -> Result<()> {
    let socket_path = "/tmp/akon-daemon.sock";

    // Create socket for IPC before fork
    let listener = UnixListener::bind(socket_path)?;

    let daemonize = Daemonize::new()
        .pid_file("/tmp/akon-daemon.pid")
        .working_directory("/tmp")
        .umask(0o027);

    match daemonize.start() {
        Ok(_) => {
            // Child process: run VPN connection
            run_vpn_daemon(listener)?;
        }
        Err(e) => {
            error!("Failed to daemonize: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

fn run_vpn_daemon(listener: UnixListener) -> Result<()> {
    // 1. Establish OpenConnect connection
    let vpn = openconnect::connect()?;

    // 2. Signal parent process: connection successful
    signal_parent(ConnectionStatus::Connected)?;

    // 3. Enter event loop: handle IPC commands + monitor connection
    loop {
        for stream in listener.incoming() {
            handle_command(stream?)?;
        }
    }
}
```

**Key Patterns**:

- Check PID file existence before spawning (prevent duplicates)
- Use SIGTERM for graceful shutdown
- Clean up socket and PID file in Drop impl

**References**:

- `daemonize` docs: <https://docs.rs/daemonize/>
- Unix socket IPC: <https://doc.rust-lang.org/std/os/unix/net/struct.UnixStream.html>

---

## 8. Configuration Management

### Decision

Use `serde` + `toml` crates to parse TOML config files, with `directories` crate for XDG Base Directory compliance (`~/.config/akon/config.toml`).

### Rationale

- **Standard Format**: TOML is human-readable, well-suited for configuration
- **Type Safety**: Serde derives prevent parsing errors
- **XDG Compliance**: `directories` crate handles cross-platform config paths
- **Validation**: Custom `impl Deserialize` enables validation during parsing

### Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| JSON config | Less human-friendly, no comments support |
| YAML config | More complex parser, whitespace-sensitive |
| INI files | Limited nesting support, less expressive |

### Implementation Notes

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use directories::ProjectDirs;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub vpn_server: String,
    pub username: String,
    pub protocol: VpnProtocol,

    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VpnProtocol {
    Ssl,
    Nc,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        let contents = std::fs::read_to_string(&config_path)
            .context("Failed to read config file")?;

        let config: Config = toml::from_str(&contents)
            .context("Failed to parse TOML config")?;

        config.validate()?;
        Ok(config)
    }

    pub fn config_path() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from("", "", "akon")
            .context("Failed to determine config directory")?;

        let config_dir = project_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;

        Ok(config_dir.join("config.toml"))
    }

    fn validate(&self) -> Result<()> {
        // URL validation
        url::Url::parse(&self.vpn_server)
            .context("Invalid VPN server URL")?;

        // Username validation
        if self.username.is_empty() {
            anyhow::bail!("Username cannot be empty");
        }

        Ok(())
    }
}
```

**Key Patterns**:

- Separate public config (TOML) from secrets (keyring)
- Support `--config` CLI flag for testing
- Provide example config in docs

**References**:

- `serde` docs: <https://docs.rs/serde/>
- `toml` docs: <https://docs.rs/toml/>
- `directories` docs: <https://docs.rs/directories/>

---

## Summary

All technology choices are finalized and aligned with project constraints:

| Component | Technology | Status |
|-----------|-----------|--------|
| VPN Connection | FFI to libopenconnect via bindgen | ✅ Researched |
| Credential Storage | `secret-service` (GNOME Keyring) | ✅ Researched |
| OTP Generation | `totp-lite` crate | ✅ Researched |
| Error Handling | `thiserror` + `anyhow` | ✅ Researched |
| CLI Framework | `clap` v4 derive | ✅ Researched |
| Logging | `tracing` + `tracing-journald` | ✅ Researched |
| Daemon Process | `daemonize` + Unix sockets | ✅ Researched |
| Configuration | `serde` + `toml` + `directories` | ✅ Researched |

**No NEEDS CLARIFICATION items remaining.** Ready to proceed to Phase 1 (Design & Contracts).
