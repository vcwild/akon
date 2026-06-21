//! Integration tests for the Test Actors Framework (spec 005).
//!
//! These tests demonstrate that akon's real-world connection behavior can be
//! validated **entirely offline** — no root, no real `openconnect`, no real
//! network, and with zero impact on the host's internet access. They run under
//! a plain `cargo test`.
//!
//! Every assertion is expressed in the backend-agnostic `LifecycleEvent`
//! vocabulary, so this suite will remain valid after the `openconnect`
//! dependency is replaced by a native backend (see US4 equivalence test).
//!
//! The framework lives behind the `test-actors` feature, so this whole test
//! file is gated on it. Run with: `cargo test -p akon-core --features test-actors`.
#![cfg(feature = "test-actors")]

use std::net::IpAddr;

use akon_core::vpn::backend::{
    BackendError, ConnectionHandle, Credentials, DisconnectReason, FailureKind, LifecycleEvent,
    VpnBackend,
};
use akon_core::vpn::testkit::{
    NetworkActor, ScenarioBuilder, SimulatedBackend, TestHarness, VpnServerActor,
};
use tokio::sync::mpsc::{self, UnboundedReceiver};

fn ip() -> IpAddr {
    "10.20.30.40".parse().unwrap()
}

// ---------------------------------------------------------------------------
// User Story 1 — connection lifecycle offline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_successful_connect_then_disconnect() {
    // Given: a backend scripted for a fully successful connection.
    let server = VpnServerActor::successful_connect(ip(), "tun0");
    let backend = SimulatedBackend::new(server);
    let registry = backend.registry();
    let mut harness = TestHarness::new(backend);

    // When: we run a connect + disconnect scenario through the harness.
    let scenario = ScenarioBuilder::new().connect().disconnect().build();
    let timeline = harness.run(scenario).await;

    // Then: the observed lifecycle reaches Connected in order...
    timeline.assert_subsequence(&[
        LifecycleEvent::Connecting,
        LifecycleEvent::Authenticating,
        LifecycleEvent::Connected {
            ip: ip(),
            device: "tun0".into(),
        },
    ]);

    // ...and after disconnect the backend reports the tunnel as torn down.
    assert!(
        !harness.backend().is_alive(),
        "backend should not be alive after disconnect"
    );
    // The tunnel handle exists and is terminated in the registry (no real kill).
    let handle = harness.backend().handle().expect("a handle was assigned");
    assert!(
        !registry.is_alive(handle),
        "tunnel should be terminated, not alive"
    );
}

#[tokio::test]
async fn test_auth_failure_never_connects() {
    // Given: a backend scripted to fail authentication.
    let server = VpnServerActor::auth_failure("invalid PIN+OTP");
    let backend = SimulatedBackend::new(server);
    let mut harness = TestHarness::new(backend);

    // When: we attempt to connect.
    let scenario = ScenarioBuilder::new().connect().build();
    let timeline = harness.run(scenario).await;

    // Then: the flow ends in an authentication failure and never connects.
    timeline.assert_reached(&LifecycleEvent::Failed {
        kind: FailureKind::Authentication,
        detail: String::new(), // matched by label
    });
    timeline.assert_never(&LifecycleEvent::Connected {
        ip: ip(),
        device: "tun0".into(),
    });
    assert!(
        !harness.backend().is_alive(),
        "no tunnel should be alive after auth failure"
    );
}

// ---------------------------------------------------------------------------
// User Story 2 — network interruption + reconnection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_network_interruption_triggers_reconnect() {
    // Given: a successful connection and a network that drops then recovers.
    let server = VpnServerActor::successful_connect(ip(), "tun0");
    let backend = SimulatedBackend::new(server);
    let mut harness = TestHarness::new(backend);

    // When: stay healthy, drop the network, then expect recovery.
    let scenario = ScenarioBuilder::new()
        .connect()
        .stay_healthy(2)
        .drop_network(2)
        .expect_reconnect()
        .build();
    let timeline = harness.run(scenario).await;

    // Then: we observe the degrade -> reconnect -> connected recovery cycle.
    timeline.assert_subsequence(&[
        LifecycleEvent::Connected {
            ip: ip(),
            device: "tun0".into(),
        },
        LifecycleEvent::HealthDegraded,
        LifecycleEvent::Reconnecting { attempt: 1 },
        LifecycleEvent::Connected {
            ip: ip(),
            device: "tun0".into(),
        },
    ]);
}

#[tokio::test]
async fn test_steady_healthy_never_reconnects() {
    // Given: a healthy connection that stays up.
    let server = VpnServerActor::successful_connect(ip(), "tun0");
    let backend = SimulatedBackend::new(server);
    let mut harness = TestHarness::new(backend);

    // When: we stay healthy for several polls (explicit reachable network).
    let scenario = ScenarioBuilder::new()
        .connect()
        .network(NetworkActor::reachable())
        .stay_healthy(5)
        .build();
    let timeline = harness.run(scenario).await;

    // Then: no degradation or reconnection ever occurs.
    timeline.assert_never(&LifecycleEvent::HealthDegraded);
    timeline.assert_never(&LifecycleEvent::Reconnecting { attempt: 1 });
}

// ---------------------------------------------------------------------------
// User Story 3 — declarative scenario authoring + recorded timeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_scenario_builder_records_ordered_timeline() {
    let server = VpnServerActor::successful_connect(ip(), "tun9");
    let backend = SimulatedBackend::new(server);
    let mut harness = TestHarness::new(backend);

    let scenario = ScenarioBuilder::new()
        .connect()
        .stay_healthy(1)
        .drop_network(1)
        .expect_reconnect()
        .disconnect()
        .build();
    let timeline = harness.run(scenario).await;

    // The recorded timeline is non-empty and opens with Connecting.
    assert!(!timeline.events().is_empty());
    assert_eq!(timeline.events().first(), Some(&LifecycleEvent::Connecting));
    // And the full real-world arc is present in order.
    timeline.assert_subsequence(&[
        LifecycleEvent::Connecting,
        LifecycleEvent::Connected {
            ip: ip(),
            device: "tun9".into(),
        },
        LifecycleEvent::HealthDegraded,
        LifecycleEvent::Reconnecting { attempt: 1 },
    ]);
}

// ---------------------------------------------------------------------------
// User Story 4 — same scenario, swappable backend, equivalent behavior
// ---------------------------------------------------------------------------

/// A second, independent `VpnBackend` implementation used to prove the harness
/// and scenarios are genuinely backend-agnostic. It emits the same observable
/// lifecycle as a successful connect, but via a completely different internal
/// mechanism (a hand-rolled event stream rather than a server actor).
///
/// This stands in for a *future native backend*: when one is written, it will
/// be validated by the very same scenario suite with no changes here.
struct AlternateBackend {
    alive: bool,
    handle: Option<ConnectionHandle>,
}

impl AlternateBackend {
    fn new() -> Self {
        Self {
            alive: false,
            handle: None,
        }
    }
}

impl VpnBackend for AlternateBackend {
    fn connect(
        &mut self,
        _credentials: Credentials,
    ) -> Result<UnboundedReceiver<LifecycleEvent>, BackendError> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.alive = true;
        self.handle = Some(ConnectionHandle(7777));
        // Emit an equivalent successful-connect lifecycle.
        let _ = tx.send(LifecycleEvent::Connecting);
        let _ = tx.send(LifecycleEvent::Authenticating);
        let _ = tx.send(LifecycleEvent::SessionEstablished);
        let _ = tx.send(LifecycleEvent::LinkUp {
            ip: ip(),
            device: "tun0".into(),
        });
        let _ = tx.send(LifecycleEvent::Connected {
            ip: ip(),
            device: "tun0".into(),
        });
        Ok(rx)
    }

    fn disconnect(&mut self) -> Result<(), BackendError> {
        self.alive = false;
        self.handle = None;
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive
    }

    fn handle(&self) -> Option<ConnectionHandle> {
        self.handle
    }
}

/// Run one scenario against an arbitrary backend and return the lifecycle
/// labels for equivalence comparison.
async fn run_labels<B: VpnBackend>(backend: B) -> Vec<String> {
    let mut harness = TestHarness::new(backend);
    let scenario = ScenarioBuilder::new()
        .connect()
        .stay_healthy(1)
        .drop_network(1)
        .expect_reconnect()
        .build();
    let timeline = harness.run(scenario).await;
    timeline
        .events()
        .iter()
        .map(|e| e.label().to_string())
        .collect()
}

#[tokio::test]
async fn test_same_scenario_two_backends_equivalent() {
    // The SAME scenario, run against two completely different backends...
    let sim_labels = run_labels(SimulatedBackend::new(VpnServerActor::successful_connect(
        ip(),
        "tun0",
    )))
    .await;
    let alt_labels = run_labels(AlternateBackend::new()).await;

    // ...produces an equivalent observable lifecycle. This is the migration
    // safety guarantee: a replacement backend can be proven equivalent before
    // it becomes the default, enabling removal of the openconnect dependency.
    assert_eq!(
        sim_labels, alt_labels,
        "two backends produced different lifecycles:\n sim: {:?}\n alt: {:?}",
        sim_labels, alt_labels
    );

    // Sanity: the equivalent arc actually contains the real-world recovery.
    assert!(sim_labels.contains(&"HealthDegraded".to_string()));
    assert!(sim_labels.contains(&"Reconnecting".to_string()));
}

// ---------------------------------------------------------------------------
// Safety net — disconnect on an already-dead tunnel is a no-op success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_disconnect_is_idempotent() {
    let server = VpnServerActor::successful_connect(ip(), "tun0");
    let mut backend = SimulatedBackend::new(server);

    // Drive connect to completion so a handle is assigned.
    let mut rx = backend
        .connect(Credentials::new("u", "p"))
        .expect("connect starts");
    while let Some(e) = rx.recv().await {
        if e.is_terminal() || matches!(e, LifecycleEvent::Connected { .. }) {
            break;
        }
    }

    // First disconnect tears down; second is a harmless no-op.
    assert!(backend.disconnect().is_ok());
    assert!(backend.disconnect().is_ok());
    assert!(!backend.is_alive());
    // The reason vocabulary is backend-agnostic and available.
    assert!(DisconnectReason::UserRequested.is_user_requested());
}
