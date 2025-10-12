.PHONY: install-deps build run-vpn-on run-vpn-off run-vpn-status test clean logs help

# Default target
all: build

# Install system dependencies required for the project
install-deps:
	sudo dnf install -y openconnect-devel dbus-devel pkgconf-pkg-config

# Build the project in release mode
build:
	cargo build --release

# Build in debug mode
build-debug:
	cargo build

# Run VPN connection (with logs)
run-vpn-on:
	RUST_LOG=akon=trace,akon_core=trace cargo run --release -- vpn on

# Disconnect VPN
run-vpn-off:
	cargo run --release -- vpn off

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
	sudo journalctl --since "5 minutes ago" | grep -E "akon|openconnect" | tail -50

# Show segfault/crash logs
crash-logs:
	sudo journalctl --since "10 minutes ago" | grep -E "segfault|SEGV|core dump" | tail -20

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
	@echo "  make install-deps    - Install system dependencies"
	@echo "  make build          - Build in release mode"
	@echo "  make build-debug    - Build in debug mode"
	@echo "  make run-vpn-on     - Connect VPN (foreground with logs)"
	@echo "  make run-vpn-off    - Disconnect VPN"
	@echo "  make run-vpn-status - Check VPN status"
	@echo "  make test           - Run tests"
	@echo "  make logs           - Show recent logs"
	@echo "  make crash-logs     - Show crash/segfault logs"
	@echo "  make coredump       - Analyze core dumps"
	@echo "  make clean          - Clean build artifacts"
