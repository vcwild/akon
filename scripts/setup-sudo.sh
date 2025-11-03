#!/bin/bash
# Setup passwordless sudo for openconnect
# This allows akon to run openconnect without requiring password prompts

set -e

echo "Setting up passwordless sudo for openconnect..."

# Check if openconnect is installed
if ! command -v openconnect &> /dev/null; then
    echo "ERROR: openconnect is not installed"
    echo "Please install it first: sudo apt install openconnect (Debian/Ubuntu)"
    exit 1
fi

# Get the full path to openconnect
OPENCONNECT_PATH=$(which openconnect)
echo "Found openconnect at: $OPENCONNECT_PATH"

# Create sudoers configuration
SUDOERS_FILE="/etc/sudoers.d/akon"
SUDOERS_CONTENT="# Allow $USER to run openconnect without password for akon VPN
$USER ALL=(root) NOPASSWD: $OPENCONNECT_PATH"

# Write the configuration
echo "Creating sudoers configuration at $SUDOERS_FILE..."
echo "$SUDOERS_CONTENT" | sudo tee "$SUDOERS_FILE" > /dev/null

# Set proper permissions (required for sudoers files)
sudo chmod 0440 "$SUDOERS_FILE"

# Verify the configuration
if sudo visudo -c -f "$SUDOERS_FILE" 2>&1 | grep -q "parsed OK"; then
    echo "✓ Sudoers configuration installed successfully"
    echo ""
    echo "You can now run 'akon vpn on' without password prompts!"
else
    echo "ERROR: Invalid sudoers configuration"
    sudo rm -f "$SUDOERS_FILE"
    exit 1
fi
