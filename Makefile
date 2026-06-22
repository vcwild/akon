.PHONY: all install install-dev deps build-deb build-rpm package-all

# Default target - build release binary
all:
	cargo build --release

# Install release version and grant the CAP_NET_ADMIN file capability.
# akon runs as your user (keyring intact); the only privilege it needs is
# CAP_NET_ADMIN for the TUN device + netlink route setup. No openconnect, no
# passwordless sudo.
install: all
	@echo "Installing akon..."
	sudo install -m 755 target/release/akon /usr/local/bin/akon
	@echo "✓ Installed to /usr/local/bin/akon"
	@echo ""
	@echo "Removing any legacy passwordless-sudo config from older akon versions..."
	@sudo rm -f /etc/sudoers.d/akon 2>/dev/null || true
	@echo "Granting CAP_NET_ADMIN to the akon binary (setcap)..."
	@if ! command -v setcap &> /dev/null; then \
		echo "ERROR: 'setcap' not found. Install libcap:"; \
		echo "  Ubuntu/Debian: sudo apt install libcap2-bin"; \
		echo "  RHEL/Fedora:   sudo dnf install libcap"; \
		exit 1; \
	fi
	sudo setcap cap_net_admin+ep /usr/local/bin/akon
	@echo "✓ Granted cap_net_admin+ep to /usr/local/bin/akon"
	@echo ""
	@echo "Installing polkit rule so VPN DNS applies without password prompts..."
	sudo install -d -m 755 /usr/share/polkit-1/rules.d
	sudo install -m 644 packaging/polkit/49-akon-resolved-dns.rules /usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules
	@echo "✓ Installed /usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules"
	@echo ""
	@echo "Installation complete! Run akon as your normal user (no sudo):"
	@echo "  akon setup"
	@echo "  akon vpn on"

# Remove akon, its capability, and the polkit rule.
.PHONY: uninstall
uninstall:
	@echo "Removing akon..."
	sudo rm -f /usr/local/bin/akon
	sudo rm -f /usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules
	sudo rm -f /etc/sudoers.d/akon 2>/dev/null || true
	@echo "✓ Removed akon, polkit rule, and any legacy sudoers config"

# Install development version for debugging
install-dev:
	cargo build
	@echo "Installing debug akon..."
	sudo install -m 755 target/debug/akon /usr/local/bin/akon-dev
	@echo "✓ Installed to /usr/local/bin/akon-dev"
	@echo ""
	@echo "You can now run:"
	@echo "  akon-dev setup"

.PHONY: deps
# Install system dependencies for building/running akon on common Linux runners.
# Supports Ubuntu/Debian (apt) and Fedora/RHEL (dnf/yum). If sudo is not available
# or the distro is not detected, the target will print the manual commands to run.
deps:
	@echo "Checking system for package manager and distro..."
	@sh -c '\
	if [ -f /etc/os-release ]; then . /etc/os-release; fi; \
	SUDO=""; if [ "$$(id -u)" -ne 0 ]; then \
		if command -v sudo >/dev/null 2>&1; then SUDO=sudo; else SUDO=; fi; \
	fi; \
	if [ -n "$$SUDO" ]; then \
		echo "Using sudo to install packages"; \
	fi; \
	case "$$ID" in \
		ubuntu|debian|linuxmint|pop) \
			if [ -z "$$SUDO" ]; then \
				echo "Detected $$ID (Ubuntu/Debian)."; \
				echo "Run as root or ensure 'sudo' is available and re-run:"; \
				echo "  sudo apt-get update && sudo apt-get install -y libcap2-bin libdbus-1-dev pkg-config"; \
				exit 0; \
			fi; \
			echo "Installing libcap (setcap), dbus dev, and pkg-config (apt)..."; \
			$$SUDO apt-get update && $$SUDO apt-get install -y libcap2-bin libdbus-1-dev pkg-config; \
			;; \
		fedora|rhel|centos) \
			if [ -z "$$SUDO" ]; then \
				echo "Detected $$ID (Fedora/RHEL)."; \
				echo "Run as root or ensure 'sudo' is available and re-run:"; \
				echo "  sudo dnf install -y libcap dbus-devel pkgconf-pkg-config"; \
				exit 0; \
			fi; \
			echo "Installing libcap (setcap), dbus dev, and pkg-config (dnf/yum)..."; \
			if command -v dnf >/dev/null 2>&1; then \
				$$SUDO dnf install -y libcap dbus-devel pkgconf-pkg-config; \
			else \
				$$SUDO yum install -y libcap dbus-devel pkgconf-pkg-config; \
			fi; \
			;; \
		*) \
			echo "Could not detect a supported distro (ID=$$ID)."; \
			echo "Please run one of the following commands manually depending on your distro:"; \
			echo "  Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y libcap2-bin libdbus-1-dev pkg-config"; \
			echo "  Fedora/RHEL:   sudo dnf install -y libcap dbus-devel pkgconf-pkg-config"; \
			exit 0; \
		;; \
	esac'

# Build .deb package for Ubuntu/Debian
build-deb: all
	@echo "Building .deb package..."
	@if ! command -v cargo-deb &> /dev/null; then \
		echo "Installing cargo-deb..."; \
		cargo install cargo-deb; \
	fi
	cargo deb --no-build
	@echo "✓ Package created: $$(ls -1 target/debian/*.deb | tail -1)"
	@echo ""
	@echo "Install with:"
	@echo "  sudo dpkg -i $$(ls -1 target/debian/*.deb | tail -1)"

# Build .rpm package for Fedora/RHEL
build-rpm: all
	@echo "Building .rpm package..."
	@if ! command -v cargo-generate-rpm &> /dev/null; then \
		echo "Installing cargo-generate-rpm..."; \
		cargo install cargo-generate-rpm; \
	fi
	cargo generate-rpm
	@echo "✓ Package created: $$(ls -1 target/generate-rpm/*.rpm | tail -1)"
	@echo ""
	@echo "Install with:"
	@echo "  sudo rpm -i $$(ls -1 target/generate-rpm/*.rpm | tail -1)"

# Build both packages
package-all: build-deb build-rpm
	@echo ""
	@echo "All packages built successfully!"
	@echo "DEB: $$(ls -1 target/debian/*.deb | tail -1)"
	@echo "RPM: $$(ls -1 target/generate-rpm/*.rpm | tail -1)"
