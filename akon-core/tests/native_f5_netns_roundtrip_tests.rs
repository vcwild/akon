//! Network-namespace data-plane ROUND-TRIP regression test.
//!
//! This is the regression lock for two production data-plane bugs found via the
//! `f5_dataplane_probe`:
//!
//!  1. `LinuxTun` used `tokio::fs::File` (buffered, offset-tracked I/O) for the
//!     TUN. A TUN is a packet device, so packets just *written* were read back
//!     immediately — an echo/loop that hung the real VPN. The fix uses
//!     `AsyncFd` + raw `read(2)`/`write(2)` syscalls (one packet per syscall).
//!  2. `Connected` was emitted before the interface was configured, and
//!     `configure()` errors were swallowed (the tunnel looked up but was dead).
//!
//! The probe brings up the **real `LinuxTun`** against an in-process F5 echo
//! server (which swaps IP src/dst + UDP ports), sends a UDP datagram through the
//! tunnel, and asserts the echo is **delivered back to a local socket** — i.e.
//! a genuine end-to-end data-plane round-trip with no looping.
//!
//! It runs entirely inside a throwaway **network namespace** (`unshare -rn`) so
//! it has ZERO effect on the host's networking even though it installs
//! full-tunnel routes. It is gated and self-skips unless:
//!   - `AKON_RUN_TUN_TESTS=1` is set, and
//!   - `unshare` with user+net namespaces is available here.
//!
//! Run with:
//!   AKON_RUN_TUN_TESTS=1 cargo test -p akon-core --features test-actors \
//!     --test native_f5_netns_roundtrip_tests -- --nocapture
#![cfg(all(feature = "test-actors", target_os = "linux"))]

use std::path::PathBuf;
use std::process::Command;

fn enabled() -> bool {
    std::env::var("AKON_RUN_TUN_TESTS").as_deref() == Ok("1")
}

/// Locate the compiled `f5_dataplane_probe` binary next to the test executable
/// (cargo builds required-feature bins into the same target profile dir).
fn probe_binary() -> Option<PathBuf> {
    // current_exe -> target/<profile>/deps/<test>-<hash>; bins live two levels up.
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?; // .../deps
    let profile_dir = deps.parent()?; // .../<profile>
    let cand = profile_dir.join("f5_dataplane_probe");
    cand.exists().then_some(cand)
}

/// Can we create a user+net namespace here? (rootless via `unshare -rn`).
fn netns_available() -> bool {
    Command::new("unshare")
        .args(["-rn", "--", "true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn dataplane_round_trips_through_real_tun_in_netns() {
    if !enabled() {
        eprintln!("skipping: set AKON_RUN_TUN_TESTS=1 to run the netns round-trip test");
        return;
    }
    if !netns_available() {
        eprintln!("skipping: `unshare -rn` (user+net namespaces) not available here");
        return;
    }
    let probe = match probe_binary() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: f5_dataplane_probe binary not found; build it with \
                 `cargo build -p akon-core --features test-actors --bin f5_dataplane_probe`"
            );
            return;
        }
    };

    // Run the probe inside a fresh user+net namespace with its own loopback and
    // a lo default, so full-tunnel routing is fully isolated from the host. The
    // probe refuses to run unless AKON_PROBE_ISOLATED=1 is set (set ONLY here,
    // inside the throwaway netns) — it can never touch a real host.
    let script = format!(
        "ip link set lo up; ip route add default dev lo 2>/dev/null || true; \
         exec env AKON_PROBE_ISOLATED=1 {}",
        probe.display()
    );
    let output = Command::new("unshare")
        .args(["-rn", "--map-root-user", "bash", "-c", &script])
        .output()
        .expect("spawn unshare");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- probe stdout ---\n{stdout}\n--- probe stderr (tail) ---");
    for line in stderr
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .iter()
        .rev()
    {
        eprintln!("{line}");
    }

    assert!(
        stdout.contains("RESULT: ok"),
        "data-plane round-trip failed (no `RESULT: ok`); exit={:?}\nstdout:\n{stdout}",
        output.status.code()
    );
    // The probe also exercises the host-teardown reconciler and prints
    // `TEARDOWN: ok` once it has verified the interface + routes are gone —
    // proving `akon vpn off` fully restores host networking.
    assert!(
        stderr.contains("TEARDOWN: ok"),
        "host teardown did not fully restore state (no `TEARDOWN: ok`)\nstderr:\n{stderr}"
    );
    assert!(
        output.status.success(),
        "probe exited non-zero: {:?}",
        output.status.code()
    );
}
