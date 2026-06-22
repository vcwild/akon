#!/bin/sh
# RPM post-uninstall script for akon
# Removes sudoers configuration file

SUDOERS_FILE="/etc/sudoers.d/akon"

# Remove sudoers file on uninstall (not on upgrade)
if [ $1 -eq 0 ]; then
    rm -f "$SUDOERS_FILE"
    # The polkit rule is a packaged asset (rpm removes it); also remove it here
    # in case it was modified locally.
    rm -f /usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules
    echo "akon configuration has been removed."

    # Clean up any temporary state files
    rm -f /tmp/akon_vpn_state.json 2>/dev/null || true
fi

exit 0
