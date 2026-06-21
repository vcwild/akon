#!/usr/bin/env bash
# Rootless data-plane validation, fully containerized.
#
# Builds the `f5_dataplane_probe` into an image that grants the binary the
# `cap_net_admin+ep` FILE CAPABILITY and runs it as a NON-ROOT user, then runs a
# full native data-plane round-trip (real TUN + in-process netlink routing) and
# teardown INSIDE a container. This proves the openconnect rootless feature
# parity with ZERO effect on the host (no sudo, no host networking touched).
#
# Usage:
#   ./test-support/run-rootless-validation.sh
#
# Requires: podman. The test self-skips if podman is unavailable.

set -euo pipefail
cd "$(dirname "$0")/.."

exec env AKON_RUN_PODMAN_TESTS=1 \
  cargo test -p akon-core --features test-actors \
  --test native_f5_podman_tests rootless_dataplane_runs_in_container_as_user \
  -- --nocapture --test-threads=1
