#!/usr/bin/env bash
# Run the production data-plane sign-off WITHOUT cargo on root's PATH AND without
# root needing your keyring.
#
# It (1) builds the test binary AND the akon CLI as your user, (2) generates the
# PIN+OTP password as your user (via `akon get-password`, which reads YOUR
# keyring), then (3) runs only the test binary under `sudo -E` for the TUN,
# passing the pre-generated password via AKON_SOAK_PASSWORD. The password is
# never printed and is only held in the environment of the elevated test.
#
# This is the interim model: the elevated step needs root only for the TUN; the
# credential is produced unprivileged. (The follow-up rootless model uses a
# CAP_NET_ADMIN file capability + in-process netlink so no sudo is needed at all.)
#
# Usage:
#   AKON_SOAK_PROBE_TARGET=intranet.example.com ./test-support/run-dataplane-signoff.sh
#
# Required: AKON_SOAK_PROBE_TARGET  host/host:port/URL reachable only via the VPN.
# Optional: AKON_F5_DEBUG=1         verbose tracing.

set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${AKON_SOAK_PROBE_TARGET:-}" ]]; then
  echo "ERROR: set AKON_SOAK_PROBE_TARGET to a host reachable only via the VPN."
  echo "  e.g. AKON_SOAK_PROBE_TARGET=intranet.example.com $0"
  exit 2
fi

export AKON_SIGNOFF_PRODUCTION=1
export AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION
export AKON_F5_DEBUG="${AKON_F5_DEBUG:-1}"

echo ">> Building akon CLI + sign-off test binary (as $USER)..."
cargo build --bin akon >/dev/null
BIN=$(cargo test --test production_dataplane_signoff_test --no-run --message-format=json 2>/dev/null \
  | sed -n 's/.*"executable":"\([^"]*production_dataplane_signoff_test[^"]*\)".*/\1/p' \
  | tail -1)
if [[ -z "${BIN:-}" || ! -x "$BIN" ]]; then
  echo "ERROR: could not locate the built test binary."
  exit 1
fi

echo ">> Generating PIN+OTP as your user (reads YOUR keyring)..."
# Capture without echoing; abort if it fails so we never run with an empty pass.
if ! AKON_SOAK_PASSWORD=$(cargo run --quiet --bin akon -- get-password); then
  echo "ERROR: 'akon get-password' failed — is your keyring set up (akon setup)?"
  exit 1
fi
if [[ -z "${AKON_SOAK_PASSWORD}" ]]; then
  echo "ERROR: generated password was empty."
  exit 1
fi
export AKON_SOAK_PASSWORD

echo ">> Test binary: $BIN"
echo ">> Running under sudo (TUN needs CAP_NET_ADMIN); password passed via env, not printed."
exec sudo -E "$BIN" --nocapture --test-threads=1
