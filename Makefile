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
	@echo "Configuring passwordless sudo for openconnect..."
	@if ! command -v openconnect &> /dev/null; then \
		echo "ERROR: openconnect is not installed"; \
		echo "Please install it first:"; \
		echo "  Ubuntu/Debian: sudo apt install openconnect"; \
		echo "  RHEL/Fedora:   sudo dnf install openconnect"; \
		exit 1; \
	fi
	@OPENCONNECT_PATH=$$(which openconnect); \
	SUDOERS_FILE="/etc/sudoers.d/akon"; \
	echo "# Allow $$USER to run openconnect without password for akon VPN" | sudo tee $$SUDOERS_FILE > /dev/null; \
	echo "$$USER ALL=(root) NOPASSWD: $$OPENCONNECT_PATH" | sudo tee -a $$SUDOERS_FILE > /dev/null; \
	sudo chmod 0440 $$SUDOERS_FILE; \
	if sudo visudo -c -f $$SUDOERS_FILE 2>&1 | grep -q "parsed OK"; then \
		echo "✓ Passwordless sudo configured for openconnect"; \
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
