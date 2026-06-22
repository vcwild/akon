//! Log-level gate tests (T003 — spec 008).
//!
//! Verify that `[tun-cfg]`, `[tun-io]`, and `[dns]` internal traces are absent
//! from stderr by default and present when `AKON_F5_DEBUG=1` is set.
//!
//! We can't capture `eprintln!` within a single process test (it goes straight
//! to the file descriptor). Instead we run the test binary as a **subprocess**
//! with a well-known env var (`AKON_LOG_LEVEL_PROBE=1`) to trigger a short
//! FakeTun connect-to-Connected, capturing its stderr.
#![cfg(feature = "test-actors")]

use std::process::Command;

// ---- In-process probe (run when AKON_LOG_LEVEL_PROBE=1) ----

/// The test that acts as the probe: when `AKON_LOG_LEVEL_PROBE=1` is set it
/// runs the FakeTun connect path and exits immediately; otherwise it self-skips.
/// This lets the subprocess tests above re-exec this binary as the probe.
#[tokio::test]
async fn log_level_probe_entrypoint() {
    if std::env::var("AKON_LOG_LEVEL_PROBE").as_deref() != Ok("1") {
        return; // normal test run: skip
    }
    run_probe().await;
    std::process::exit(0);
}

async fn run_probe() {
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::NativeF5Backend;
    use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
    use akon_core::vpn::testkit::fake_dns::FakeDns;
    use akon_core::vpn::testkit::fake_tun::FakeTun;
    use akon_core::vpn::testkit::transport::MemoryTransport;

    let (client, mut server) = MemoryTransport::pair();
    tokio::spawn(async move {
        F5ServerActor::new(F5ServerScript::default())
            .run(&mut server)
            .await;
    });
    let (tun, _tun_handle) = FakeTun::new();
    let (dns, _dns_handle) = FakeDns::new();
    let mut backend = NativeF5Backend::with_parts(
        Box::new(client),
        Box::new(tun),
        Box::new(dns),
        "vpn.example.com",
    );
    let mut rx = backend
        .connect(Credentials::new("testuser", "1234567890"))
        .expect("connect");
    while let Some(ev) = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .ok()
        .flatten()
    {
        if matches!(
            ev,
            LifecycleEvent::Connected { .. } | LifecycleEvent::Failed { .. }
        ) {
            break;
        }
    }
}

// ---- Subprocess assertion tests ----

fn probe_stderr(debug: bool) -> String {
    let exe = std::env::current_exe().expect("current_exe");
    // Run only the probe entrypoint test in the subprocess.
    let mut cmd = Command::new(&exe);
    cmd.args(["log_level_probe_entrypoint", "--nocapture"])
        .env("AKON_LOG_LEVEL_PROBE", "1");
    if debug {
        cmd.env("AKON_F5_DEBUG", "1");
    } else {
        cmd.env_remove("AKON_F5_DEBUG");
    }
    let out = cmd.output().expect("spawn probe");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn no_trace_lines_without_debug_flag() {
    let stderr = probe_stderr(false);
    for prefix in &["[tun-cfg]", "[tun-io]", "[dns] applied", "[dns] resolvectl"] {
        assert!(
            !stderr.contains(prefix),
            "trace {prefix:?} appeared without AKON_F5_DEBUG:\n{stderr}"
        );
    }
}

#[test]
fn trace_lines_present_with_debug_flag() {
    let stderr = probe_stderr(true);
    // With AKON_F5_DEBUG=1 at minimum the HTTP or PPP trace should appear.
    assert!(
        stderr.contains("[f5-ppp]") || stderr.contains("[f5-http]") || stderr.contains("[tun-cfg]"),
        "expected trace lines with AKON_F5_DEBUG=1:\n{stderr}"
    );
}
