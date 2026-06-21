#!/usr/bin/env bash
# Bring up a REAL VPN connection using the NATIVE F5 backend (no openconnect),
# so you can actually browse intranet sites through it — the true acceptance test.
#
# Like the soak helper, it builds + generates the PIN+OTP as your user (so YOUR
# keyring is read), then runs `akon vpn on` under sudo for the TUN, passing the
# password via AKON_VPN_PASSWORD (never printed). The native path applies the
# tunnel IP, MTU, split routes, and DNS; verbose tracing shows exactly what it
# installs.
#
# Requires: ~/.config/akon/config.toml with protocol="f5". This script forces
# the native backend on for this run (it does NOT edit your config).
#
# Usage:
#   ./test-support/run-native-vpn.sh
# Then, in ANOTHER terminal, try reaching an intranet site (e.g. curl/browser).
# Ctrl-C this process to disconnect (TUN + routes are torn down).

set -euo pipefail
cd "$(dirname "$0")/.."

echo ">> Building akon (as $USER)..."
cargo build --bin akon
AKON_BIN="target/debug/akon"
LOG="/tmp/akon-native-vpn.log"
echo ">> Full output will also be saved to $LOG"

echo ">> Generating PIN+OTP as your user (reads YOUR keyring)..."
if ! AKON_VPN_PASSWORD=$("$AKON_BIN" get-password); then
  echo "ERROR: 'akon get-password' failed — is your keyring set up (akon setup)?"
  exit 1
fi
[[ -n "${AKON_VPN_PASSWORD}" ]] || { echo "ERROR: empty password"; exit 1; }
export AKON_VPN_PASSWORD

# Force the native backend on for THIS run without editing the user's config,
# by pointing akon at a temp config dir that copies the real config + flag.
SRC="${AKON_CONFIG_DIR:-$HOME/.config/akon}/config.toml"
[[ -f "$SRC" ]] || { echo "ERROR: $SRC not found (run 'akon setup')."; exit 1; }
TMPDIR_CFG=$(mktemp -d)
trap 'rm -rf "$TMPDIR_CFG"' EXIT
cp "$SRC" "$TMPDIR_CFG/config.toml"
if ! grep -q '^[[:space:]]*native_backend[[:space:]]*=' "$TMPDIR_CFG/config.toml"; then
  # Insert under the [vpn] table.
  sed -i '/^\[vpn\]/a native_backend = true' "$TMPDIR_CFG/config.toml"
else
  sed -i 's/^[[:space:]]*native_backend[[:space:]]*=.*/native_backend = true/' "$TMPDIR_CFG/config.toml"
fi
echo ">> Using native backend (temp config at $TMPDIR_CFG)."

echo ">> Running 'akon vpn on' under sudo (TUN needs CAP_NET_ADMIN)."
echo ">> In another terminal, try reaching an intranet site. Ctrl-C here to disconnect."
echo ">> (All [tun-cfg] route diagnostics are captured to $LOG)"
# Capture EVERYTHING (stdout+stderr) to the log AND the terminal so the
# routing diagnostics can't be lost to interleaving.
sudo -E env \
  AKON_CONFIG_DIR="$TMPDIR_CFG" \
  AKON_VPN_PASSWORD="$AKON_VPN_PASSWORD" \
  RUST_LOG="${RUST_LOG:-info}" \
  AKON_F5_DEBUG="${AKON_F5_DEBUG:-1}" \
  "$AKON_BIN" vpn on 2>&1 | tee "$LOG"
