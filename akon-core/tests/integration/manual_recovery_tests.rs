// Integration tests for manual recovery flows (native F5 backend).
// User Story 4: manual recovery after repeated reconnection failures.
//
// With the native backend there are NO external openconnect processes to reap:
// the VPN runs in-process, and recovery is (a) the in-process supervisor giving
// up after max_attempts, and (b) `akon vpn off`, which replays the persisted
// HostTeardownPlan to restore host networking (idempotent, works even after a
// SIGKILL), then `akon vpn on [--force]` to reconnect.
//
// These remain #[ignore]d end-to-end aspirations that need a full connection
// harness; the underlying mechanisms ARE unit/integration tested elsewhere:
//   - teardown reconciliation: akon-core teardown unit tests +
//     native_f5_netns_roundtrip_tests (TEARDOWN: ok) + native_f5_podman_tests.
//   - reaching Error after exhausted attempts: reconnection_tests / health_check.

#[test]
#[ignore = "Requires full native VPN integration harness"]
fn test_manual_recovery_after_max_attempts_exceeded() {
    // 1. Establish a native VPN connection (in-process).
    // 2. Cause repeated health-check failures to exceed max_attempts.
    // 3. Verify the supervisor reports Error and stops.
    // 4. Run `akon vpn off`: the HostTeardownPlan reconciler removes the tun,
    //    server-pin route, restores rp_filter, and reverts DNS.
    // 5. Run `akon vpn on`: a fresh connection succeeds.
    //
    // Flow: Connected → unhealthy → Reconnecting(1..max) → Error →
    //       [vpn off restores host] → [vpn on] → Connected.
}

#[test]
#[ignore = "Requires full native VPN integration harness"]
fn test_vpn_off_restores_host_after_failure() {
    // Validates the native recovery primitive in isolation:
    // 1. Connect (TUN + full-tunnel routes + VPN DNS applied).
    // 2. Run `akon vpn off`.
    // 3. Verify: tun device gone, default route restored, rp_filter restored,
    //    DNS reverted — even if the supervising process was killed (the plan is
    //    persisted in the state file and replayed out-of-process).
    //
    // The mechanics are covered by native_f5_netns_roundtrip_tests
    // (asserts `TEARDOWN: ok`) and native_f5_podman_tests.
}

#[test]
#[ignore = "Requires full native VPN integration harness"]
fn test_force_reconnect_disconnects_then_reconnects() {
    // Validates `akon vpn on --force`:
    // 1. With an active connection recorded, run `akon vpn on --force`.
    // 2. Verify it tears down the existing session (vpn off path) first, then
    //    establishes a new one.
}

#[test]
#[ignore = "Requires full native VPN integration harness"]
fn test_status_reports_stale_after_process_gone() {
    // Validates `akon vpn status` UX:
    // 1. With a state file whose recorded pid is no longer running,
    // 2. `akon vpn status` reports "inactive (stale)" and suggests `akon vpn off`
    //    to clean up — covered by vpn_status integration tests.
}
