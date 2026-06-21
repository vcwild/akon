//! PRODUCTION SIGN-OFF TEST — connects the native F5 backend to the **real**
//! VPN appliance configured in `~/.config/akon/config.toml` using the user's
//! **real keyring credentials** (PIN + OTP).
//!
//! ## ⚠️ This test hits a production network. Read before enabling.
//!
//! This is the final acceptance gate, run **once, deliberately, by a human**,
//! only after every other layer has been proven (pure protocol layers, the
//! in-memory actors, the real-local-TLS test, and the Podman Fedora/Ubuntu
//! container tests). It is therefore **disabled by default** and requires an
//! explicit double opt-in so it can never run accidentally, in CI, or as part
//! of the normal `cargo test`.
//!
//! ### What it does (and deliberately does NOT do)
//! - Loads the real config and generates the real PIN+OTP password from the
//!   keyring, then performs the full F5 handshake against the live server:
//!   auth → config → tunnel upgrade → PPP → `Connected`.
//! - Uses a **control-plane-only** backend: **no TUN device is created, no
//!   routes are installed, and no DNS is changed.** It validates reachability
//!   and protocol correctness against the real appliance **without taking over
//!   or disrupting the developer's own connectivity.**
//! - Disconnects immediately after reaching `Connected` (clean PPP terminate +
//!   F5 logout). Total contact with the server is a few seconds.
//! - It is bounded by a hard timeout so it cannot hang.
//!
//! ### How to run (the only way it executes)
//! ```text
//! AKON_SIGNOFF_PRODUCTION=1 \
//! AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION \
//! cargo test --test production_signoff_test -- --nocapture --test-threads=1
//! ```
//! Requires: a populated `~/.config/akon/config.toml` (protocol `f5`) and the
//! PIN + OTP secret stored in the keyring for the configured username.

use std::time::Duration;

const ACK_PHRASE: &str = "I_UNDERSTAND_THIS_HITS_PRODUCTION";

/// Whether the sign-off test is explicitly, doubly authorized to run.
fn signoff_authorized() -> bool {
    std::env::var("AKON_SIGNOFF_PRODUCTION").as_deref() == Ok("1")
        && std::env::var("AKON_SIGNOFF_ACK").as_deref() == Ok(ACK_PHRASE)
}

#[tokio::test]
async fn production_signoff_native_f5_connects_to_real_appliance() {
    // ---- Hard gate: never run without explicit, acknowledged opt-in ----
    if !signoff_authorized() {
        eprintln!(
            "SKIP: production sign-off test is disabled.\n\
             To run it deliberately against the real appliance, set BOTH:\n  \
             AKON_SIGNOFF_PRODUCTION=1\n  \
             AKON_SIGNOFF_ACK={ACK_PHRASE}\n\
             (It hits a production network using your real keyring credentials.)"
        );
        return;
    }

    use akon_core::auth::password::generate_password;
    use akon_core::config::toml_config::{get_config_path, TomlConfig};
    use akon_core::config::VpnProtocol;
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::NativeF5Backend;

    // ---- Load the REAL configuration ----
    let config_path = get_config_path().expect("resolve config path");
    let toml_config = TomlConfig::from_file(&config_path)
        .expect("load ~/.config/akon/config.toml — is akon configured?");
    let config = toml_config.vpn_config;

    assert_eq!(
        config.protocol,
        VpnProtocol::F5,
        "sign-off test only applies to the F5 protocol (config has {:?})",
        config.protocol
    );
    eprintln!(
        "sign-off: target server = {} (user = {})",
        config.server, config.username
    );

    // ---- Real credentials from the keyring (PIN + OTP) ----
    let password = generate_password(&config.username)
        .expect("generate PIN+OTP from keyring — are credentials stored?");

    // ---- Connect control-plane-only (no TUN/routes/DNS side effects) ----
    eprintln!("sign-off: connecting to the live appliance (control-plane only)...");
    let mut backend = NativeF5Backend::connect_control_plane_only(&config)
        .await
        .expect("TLS connect to the real appliance");

    let credentials = Credentials::new(config.username.clone(), password.expose().to_string());
    let mut rx = backend.connect(credentials).expect("start native connect");

    // ---- Drive to Connected, bounded so it cannot hang ----
    let mut outcome: Option<Result<String, String>> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ev)) => {
                eprintln!("sign-off: lifecycle {ev:?}");
                match ev {
                    LifecycleEvent::Connected { ip, .. } => {
                        outcome = Some(Ok(ip.to_string()));
                        break;
                    }
                    LifecycleEvent::Failed { kind, detail } => {
                        outcome = Some(Err(format!("{kind:?}: {detail}")));
                        break;
                    }
                    _ => {}
                }
            }
            Ok(None) => {
                outcome = Some(Err("event stream closed before Connected".into()));
                break;
            }
            Err(_) => { /* keep waiting until the overall deadline */ }
        }
    }

    // ---- Always disconnect immediately (clean teardown), regardless of result ----
    let _ = backend.disconnect();
    // Give teardown a brief, bounded moment to send PPP terminate + logout.
    tokio::time::sleep(Duration::from_millis(500)).await;

    match outcome {
        Some(Ok(ip)) => {
            eprintln!(
                "✅ PRODUCTION SIGN-OFF PASSED: native F5 backend connected to {} \
                 and was assigned {ip}. Disconnected cleanly.",
                config.server
            );
        }
        Some(Err(e)) => panic!("production sign-off FAILED: {e}"),
        None => panic!("production sign-off FAILED: did not reach Connected within timeout"),
    }
}
