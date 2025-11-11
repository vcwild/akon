#!/bin/sh
# RPM post-installation script for akon
# Configures passwordless sudo for required commands

# Find command paths
OPENCONNECT_PATH=$(command -v openconnect 2>/dev/null || echo "")
PKILL_PATH=$(command -v pkill 2>/dev/null || echo "")
KILL_PATH=$(command -v kill 2>/dev/null || echo "/usr/bin/kill")

# Verify paths exist
if [ -z "$OPENCONNECT_PATH" ]; then
    echo "Warning: openconnect not found. Please install it: sudo dnf install openconnect"
    OPENCONNECT_PATH="/usr/sbin/openconnect"
fi

if [ -z "$PKILL_PATH" ]; then
    PKILL_PATH="/usr/bin/pkill"
fi

if [ ! -f "$KILL_PATH" ]; then
    KILL_PATH="/usr/bin/kill"
fi

# Create sudoers.d file for akon
SUDOERS_FILE="/etc/sudoers.d/akon"

cat > "$SUDOERS_FILE" << EOF
# Allow all users to run akon-required commands without password
# This file is automatically managed by the akon package
ALL ALL=(ALL) NOPASSWD: $OPENCONNECT_PATH *
ALL ALL=(ALL) NOPASSWD: $PKILL_PATH *
ALL ALL=(ALL) NOPASSWD: $KILL_PATH *
EOF

# Set proper permissions on sudoers file
chmod 0440 "$SUDOERS_FILE"

# Verify the sudoers file syntax
if ! visudo -c -f "$SUDOERS_FILE" >/dev/null 2>&1; then
    echo "Warning: sudoers file has syntax errors. Please check /etc/sudoers.d/akon"
    rm -f "$SUDOERS_FILE"
    exit 1
fi

echo "akon has been installed successfully!"
echo "The following commands are now available without sudo password:"
echo "  - openconnect ($OPENCONNECT_PATH)"
echo "  - pkill ($PKILL_PATH)"
echo "  - kill ($KILL_PATH)"
echo ""
echo "Run 'akon --help' to get started."

exit 0
