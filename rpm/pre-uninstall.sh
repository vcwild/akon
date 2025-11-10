#!/bin/sh
# RPM pre-uninstall script for akon
# Stops any running akon daemon

# Stop any running akon daemon
if command -v akon >/dev/null 2>&1; then
    akon off 2>/dev/null || true
fi

exit 0
