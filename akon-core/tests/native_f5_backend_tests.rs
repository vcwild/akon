//! End-to-end tests for the native F5 backend (spec 006), driven entirely by
//! the test actors framework as ground truth — no real server, no root, no
//! network.
//!
//! These prove the openconnect replacement reaches `Connected` against a fake
//! F5 server that speaks the real wire protocol (HTTP auth/config + `/myvpn`
//! upgrade + PPP peer using the real framing/ppp codec), and that the native
//! backend is behaviorally equivalent to the simulated backend (US4 / FR-012).
#![cfg(feature = "test-actors")]

use akon_core::vpn::backend::{Credentials, FailureKind, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::testkit::transport::MemoryTransport;
use akon_core::vpn::testkit::{SimulatedBackend, VpnServerActor};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

/// Spawn the fake F5 server on one end of an in-memory transport and return a
/// `NativeF5Backend` wired to the other end.
fn wire_native(script: F5ServerScript) -> NativeF5Backend {
    let (client, mut server) = MemoryTransport::pair();
    let actor = F5ServerActor::new(script);
    tokio::spawn(async move {
        actor.run(&mut server).await;
    });
    NativeF5Backend::with_transport(Box::new(client), "vpn.example.com")
}

/// Collect lifecycle events from a backend until a terminal event or timeout.
async fn collect(mut rx: UnboundedReceiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut events = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
            Ok(Some(e)) => {
                let terminal = matches!(
                    e,
                    LifecycleEvent::Connected { .. }
                        | LifecycleEvent::Failed { .. }
                        | LifecycleEvent::Disconnected { .. }
                );
                events.push(e);
                if terminal {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    events
}

fn labels(events: &[LifecycleEvent]) -> Vec<&'static str> {
    events.iter().map(|e| e.label()).collect()
}

// ---------------------------------------------------------------------------
// US4 - successful native connect against the fake F5 server
// ---------------------------------------------------------------------------

#[tokio::test]
async fn native_f5_reaches_connected_against_fake_server() {
    let mut backend = wire_native(F5ServerScript::default());
    let rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");
    let events = collect(rx).await;

    // The full F5 handshake completes: auth -> session -> link up -> connected.
    let ls = labels(&events);
    assert!(ls.contains(&"Connecting"), "missing Connecting: {:?}", ls);
    assert!(
        ls.contains(&"Authenticating"),
        "missing Authenticating: {:?}",
        ls
    );
    assert!(
        ls.contains(&"SessionEstablished"),
        "missing SessionEstablished: {:?}",
        ls
    );
    assert!(ls.contains(&"Connected"), "never Connected: {:?}", ls);

    // The server-assigned IP (10.20.30.40) is reflected in the Connected event.
    let connected_ip = events.iter().find_map(|e| match e {
        LifecycleEvent::Connected { ip, .. } => Some(ip.to_string()),
        _ => None,
    });
    assert_eq!(connected_ip.as_deref(), Some("10.20.30.40"));
    assert!(backend.is_alive());
    assert!(backend.handle().is_some());
}

// ---------------------------------------------------------------------------
// US4 - authentication failure: never Connected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn native_f5_auth_failure_never_connects() {
    let mut backend = wire_native(F5ServerScript::auth_failure());
    let rx = backend
        .connect(Credentials::new("alice", "wrongpass"))
        .expect("connect starts");
    let events = collect(rx).await;

    let ls = labels(&events);
    assert!(!ls.contains(&"Connected"), "should not connect: {:?}", ls);
    let failed_auth = events.iter().any(|e| {
        matches!(
            e,
            LifecycleEvent::Failed {
                kind: FailureKind::Authentication,
                ..
            }
        )
    });
    assert!(failed_auth, "expected auth failure, got: {:?}", ls);
    assert!(!backend.is_alive());
}

// ---------------------------------------------------------------------------
// US4 - tunnel upgrade rejected: terminal network failure, no Connected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn native_f5_tunnel_rejected_fails() {
    let mut backend = wire_native(F5ServerScript::tunnel_rejected(403));
    let rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");
    let events = collect(rx).await;

    let ls = labels(&events);
    assert!(!ls.contains(&"Connected"), "should not connect: {:?}", ls);
    let failed_net = events.iter().any(|e| {
        matches!(
            e,
            LifecycleEvent::Failed {
                kind: FailureKind::Network,
                ..
            }
        )
    });
    assert!(failed_net, "expected network failure, got: {:?}", ls);
}

// ---------------------------------------------------------------------------
// US4 - cross-backend equivalence: native vs simulated produce equivalent arcs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn native_and_simulated_backends_are_equivalent() {
    // Native backend against the fake F5 server.
    let mut native = wire_native(F5ServerScript::default());
    let native_events = collect(
        native
            .connect(Credentials::new("alice", "pin123456"))
            .expect("native connect"),
    )
    .await;

    // Simulated backend scripted for the equivalent successful connect to the
    // same assigned IP/device.
    let server = VpnServerActor::successful_connect("10.20.30.40".parse().unwrap(), "tun0");
    let mut sim = SimulatedBackend::new(server);
    let sim_events = collect(
        sim.connect(Credentials::new("alice", "pin123456"))
            .expect("sim connect"),
    )
    .await;

    // Both must reach Connected with the same address, demonstrating the native
    // backend is a behaviorally-equivalent drop-in (the migration guarantee).
    let native_connected = native_events.iter().find_map(|e| match e {
        LifecycleEvent::Connected { ip, .. } => Some(ip.to_string()),
        _ => None,
    });
    let sim_connected = sim_events.iter().find_map(|e| match e {
        LifecycleEvent::Connected { ip, .. } => Some(ip.to_string()),
        _ => None,
    });
    assert_eq!(native_connected, sim_connected);
    assert_eq!(native_connected.as_deref(), Some("10.20.30.40"));

    // Both arcs reach the same terminal milestone in order.
    let native_ls = labels(&native_events);
    let sim_ls = labels(&sim_events);
    assert_eq!(native_ls.last(), Some(&"Connected"));
    assert_eq!(sim_ls.last(), Some(&"Connected"));
}
