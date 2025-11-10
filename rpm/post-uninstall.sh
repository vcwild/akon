#!/bin/sh
# RPM post-uninstall script for akon
# Removes sudoers configuration file

SUDOERS_FILE="/etc/sudoers.d/akon"

# Remove sudoers file on uninstall (not on upgrade)
if [ $1 -eq 0 ]; then
    rm -f "$SUDOERS_FILE"
    echo "akon configuration has been removed."

    # Clean up any temporary state files
    rm -f /tmp/akon_vpn_state.json 2>/dev/null || true
fi

exit 0
