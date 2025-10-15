.PHONY: install-deps build install install-dev run-vpn-on run-vpn-off run-vpn-status test clean logs help uninstall

# Default target
all: build

# Install system dependencies required for the project
install-deps:
	sudo dnf install -y openconnect-devel dbus-devel pkgconf-pkg-config libcap

# Build the project in release mode
build:
	cargo build --release

# Build in debug mode
build-debug:
	cargo build

# Build in dev mode
build-dev:
	cargo build
	@echo "✓ Built successfully"

# Install release version with network capabilities (one-time setup)
# After this, you can run 'akon vpn on' without sudo
install: build
	@echo "Installing akon with network capabilities..."
	sudo install -m 755 target/release/akon /usr/local/bin/akon
	sudo setcap cap_net_admin,cap_net_raw,cap_setuid,cap_setgid+eip /usr/local/bin/akon
	@echo "✓ Installed to /usr/local/bin/akon"
	@echo "✓ Network capabilities set (CAP_NET_ADMIN, CAP_NET_RAW, CAP_SETUID, CAP_SETGID)"
	@echo ""
	@echo "You can now run without sudo:"
	@echo "  akon vpn on"

# Install development version with network capabilities
install-dev: build-debug
	@echo "Installing debug akon with network capabilities..."
	sudo install -m 755 target/debug/akon /usr/local/bin/akon-dev
	sudo setcap cap_net_admin,cap_net_raw,cap_setuid,cap_setgid+eip /usr/local/bin/akon-dev
	@echo "✓ Installed to /usr/local/bin/akon-dev"
	@echo "✓ Network capabilities set (CAP_NET_ADMIN, CAP_NET_RAW, CAP_SETUID, CAP_SETGID)"
	@echo ""
	@echo "You can now run without sudo:"
	@echo "  akon-dev vpn on"

# Uninstall akon binaries
uninstall:
	sudo rm -f /usr/local/bin/akon /usr/local/bin/akon-dev
	@echo "✓ Uninstalled akon"

# Kill any existing akon processes
kill-akon:
	-pkill -f akon || true

# Run VPN connection - RELEASE MODE (after 'make install')
# This assumes you've run 'make install' and the binary has capabilities
run-vpn-on: install
	@echo "Connecting to VPN..."
	RUST_LOG=info akon vpn on

# Run VPN connection - DEBUG MODE (for development)
# This uses the locally built binary with capabilities set temporarily
run-vpn-on-debug: build-dev
	@echo "Setting temporary capabilities on debug binary..."
	@sudo setcap cap_net_admin+eip target/debug/akon || true
	@echo "Connecting to VPN (debug mode)..."
	RUST_LOG=debug ./target/debug/akon vpn on

# Disconnect VPN (debug)
run-vpn-off-debug:
	cargo run -- vpn off

# Disconnect VPN
run-vpn-off:
	cargo run --release -- vpn off

# Check VPN status (debug)
run-vpn-status-debug:
	cargo run -- vpn status

# Check VPN status
run-vpn-status:
	cargo run --release -- vpn status

# Run tests
test:
	cargo test

# Clean build artifacts
clean:
	cargo clean

# Show recent logs from journalctl
logs:
	sudo journalctl --since "5 minutes ago" | grep -E "akon|openconnect"

# Show segfault/crash logs
crash-logs:
	sudo journalctl --since "10 minutes ago" | grep -E "segfault|SEGV|core dump"

# Analyze core dump (if available)
coredump:
	@echo "Recent core dumps:"
	@coredumpctl list | grep akon | head -5
	@echo ""
	@echo "To debug the latest crash, run:"
	@echo "  coredumpctl debug"

# Show help
help:
	@echo "Available targets:"
	@echo "  make install-deps        - Install system dependencies"
	@echo "  make build               - Build in release mode"
	@echo "  make build-debug         - Build in debug mode"
	@echo "  make build-dev          - Build in dev mode"
	@echo "  make run-vpn-on          - Connect VPN (release, with logs)"
	@echo "  make run-vpn-on-debug    - Connect VPN (debug, with logs)"
	@echo "  make run-vpn-off         - Disconnect VPN (release)"
	@echo "  make run-vpn-off-debug   - Disconnect VPN (debug)"
	@echo "  make run-vpn-status      - Check VPN status (release)"
	@echo "  make run-vpn-status-debug - Check VPN status (debug)"
	@echo "  make test                - Run tests"
	@echo "  make logs                - Show recent logs"
	@echo "  make crash-logs          - Show crash/segfault logs"
	@echo "  make coredump            - Analyze core dumps"
	@echo "  make clean               - Clean build artifacts"
