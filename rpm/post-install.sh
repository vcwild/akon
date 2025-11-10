#!/bin/sh
# RPM post-installation script for akon
# Configures passwordless sudo for required commands

OPENCONNECT_PATH=$(command -v openconnect)
PKILL_PATH=$(command -v pkill)
KILL_PATH=$(command -v kill || echo "/usr/bin/kill")

# Create sudoers.d file for akon
SUDOERS_FILE="/etc/sudoers.d/akon"

cat > "$SUDOERS_FILE" << EOF
# Allow all users to run akon-required commands without password
# This file is automatically managed by the akon package
ALL ALL=(ALL) NOPASSWD: $OPENCONNECT_PATH
ALL ALL=(ALL) NOPASSWD: $PKILL_PATH
ALL ALL=(ALL) NOPASSWD: $KILL_PATH
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
echo "  - openconnect"
echo "  - pkill"
echo "  - kill"
echo ""
echo "Run 'akon --help' to get started."

exit 0
