.PHONY: all install install-dev

# Default target - build release binary
all:
	cargo build --release

# Install release version with passwordless sudo setup
# This configures everything needed to run akon without password prompts
install: all
	@echo "Installing akon..."
	sudo install -m 755 target/release/akon /usr/local/bin/akon
	@echo "✓ Installed to /usr/local/bin/akon"
	@echo ""
	@echo "Configuring passwordless sudo for openconnect, pkill, and kill..."
	@if ! command -v openconnect &> /dev/null; then \
		echo "ERROR: openconnect is not installed"; \
		echo "Please install it first:"; \
		echo "  Ubuntu/Debian: sudo apt install openconnect"; \
		echo "  RHEL/Fedora:   sudo dnf install openconnect"; \
		exit 1; \
	fi
	@if ! command -v pkill &> /dev/null; then \
		echo "ERROR: pkill is not installed"; \
		echo "Please install procps package:"; \
		echo "  Ubuntu/Debian: sudo apt install procps"; \
		echo "  RHEL/Fedora:   sudo dnf install procps-ng"; \
		exit 1; \
	fi
	@if [ ! -x /usr/bin/kill ] && [ ! -x /bin/kill ]; then \
		echo "ERROR: kill binary not found (expected at /usr/bin/kill or /bin/kill)"; \
		echo "Please ensure coreutils package providing kill is installed."; \
		exit 1; \
	fi
	@OPENCONNECT_PATH=$$(command -v openconnect); \
	PKILL_PATH=$$(command -v pkill); \
	if [ -x /usr/bin/kill ]; then \
		KILL_PATH=/usr/bin/kill; \
	else \
		KILL_PATH=/bin/kill; \
	fi; \
	SUDOERS_FILE="/etc/sudoers.d/akon"; \
	echo "# Allow $$USER to run openconnect, pkill, and kill without password for akon VPN" | sudo tee $$SUDOERS_FILE > /dev/null; \
	echo "$$USER ALL=(root) NOPASSWD: $$OPENCONNECT_PATH" | sudo tee -a $$SUDOERS_FILE > /dev/null; \
	echo "$$USER ALL=(root) NOPASSWD: $$PKILL_PATH" | sudo tee -a $$SUDOERS_FILE > /dev/null; \
	echo "$$USER ALL=(root) NOPASSWD: $$KILL_PATH" | sudo tee -a $$SUDOERS_FILE > /dev/null; \
	sudo chmod 0440 $$SUDOERS_FILE; \
	if sudo visudo -c -f $$SUDOERS_FILE 2>&1 | grep -q "parsed OK"; then \
		echo "✓ Passwordless sudo configured for openconnect, pkill, and kill"; \
	else \
		echo "ERROR: Invalid sudoers configuration"; \
		sudo rm -f $$SUDOERS_FILE; \
		exit 1; \
	fi
	@echo ""
	@echo "Installation complete! You can now run:"
	@echo "  akon setup"

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
				echo "  sudo apt-get update && sudo apt-get install -y openconnect libdbus-1-dev pkg-config"; \
				exit 0; \
			fi; \
			echo "Installing openconnect, dbus dev, and pkg-config (apt)..."; \
			$$SUDO apt-get update && $$SUDO apt-get install -y openconnect libdbus-1-dev pkg-config; \
			;; \
		fedora|rhel|centos) \
			if [ -z "$$SUDO" ]; then \
				echo "Detected $$ID (Fedora/RHEL)."; \
				echo "Run as root or ensure 'sudo' is available and re-run:"; \
				echo "  sudo dnf install -y openconnect dbus-devel pkgconf-pkg-config"; \
				exit 0; \
			fi; \
			echo "Installing openconnect, dbus dev, and pkg-config (dnf/yum)..."; \
			if command -v dnf >/dev/null 2>&1; then \
				$$SUDO dnf install -y openconnect dbus-devel pkgconf-pkg-config; \
			else \
				$$SUDO yum install -y openconnect dbus-devel pkgconf-pkg-config; \
			fi; \
			;; \
		*) \
			echo "Could not detect a supported distro (ID=$$ID)."; \
			echo "Please run one of the following commands manually depending on your distro:"; \
			echo "  Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y openconnect libdbus-1-dev pkg-config"; \
			echo "  Fedora/RHEL:   sudo dnf install -y openconnect dbus-devel pkgconf-pkg-config"; \
			exit 0; \
		;; \
	esac'
