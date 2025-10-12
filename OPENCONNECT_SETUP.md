# OpenConnect Setup Guide

## Overview

This project uses custom FFI bindings to the OpenConnect C library with a safe, leak-free password callback pattern. While `openconnect-core` exists, its dependency `openconnect-sys` v0.1.5 has build issues (tries to build OpenConnect from source instead of using system libraries). Our implementation follows best practices for FFI safety.

## System Requirements

You need to install the OpenConnect development libraries on your system before building the project.

### Ubuntu/Debian

```bash
sudo apt-get update
sudo apt-get install -y libopenconnect-dev pkg-config
```

### Fedora/RHEL/CentOS

```bash
sudo dnf install openconnect-devel pkgconfig
```

### Arch Linux

```bash
sudo pacman -S openconnect pkg-config
```

### macOS (Homebrew)

```bash
brew install openconnect pkg-config
```

## Verification

After installation, verify that the library is available:

```bash
pkg-config --exists libopenconnect && echo "✓ OpenConnect found" || echo "✗ OpenConnect not found"
```

You can also check the version:

```bash
pkg-config --modversion libopenconnect
```

## Building the Project

Once OpenConnect is installed, you can build the project normally:

```bash
cargo build
cargo test
```

## Benefits of Our Implementation

The refactored implementation provides several improvements over the original:

1. **No Global State**: Removed the global `Mutex` for credentials
2. **No Memory Leaks**: Proper CString ownership - kept in struct, freed in Drop
3. **Password Callback Pattern**: Uses `password_cb` with `libc::strdup` for proper memory management
4. **Safer FFI**: Minimal unsafe blocks with clear ownership boundaries
5. **Better Error Handling**: Proper error propagation with Rust's `Result` type

## Implementation Details

Our custom implementation (Option A - Password Callback):

- Stores username/password as `CString` in the struct (keeps them alive)
- Uses password callback that calls `libc::strdup` (OpenConnect frees it)
- Sets `username` and `privdata` fields on `openconnect_info`
- Properly cleans up in `Drop` implementation
- No leaked CStrings or global state

**Why not `openconnect-core`?**

- The `openconnect-sys` v0.1.5 dependency tries to build OpenConnect from source
- Build script fails to find `config.h` even with system libraries installed
- Our custom bindings work reliably with system OpenConnect

See `akon-core/src/vpn/openconnect_old.rs` for reference on the safer password callback pattern we should implement.
