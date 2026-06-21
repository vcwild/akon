//! Test harness + recorded timeline + assertions.
//!
//! [`TestHarness`] is generic over any [`VpnBackend`], so a single scenario can
//! be executed against the simulated backend today and a native backend
//! tomorrow — the cornerstone of the migration-safety strategy (run the same
//! suite against the replacement and assert behavioral equivalence before
//! switching the default).
//!
//! The harness records every observed [`LifecycleEvent`] into a [`Timeline`]
//! that provides ordered sub-sequence assertions with clear failure messages.

use crate::vpn::backend::{LifecycleEvent, VpnBackend};
use crate::vpn::testkit::network_actor::Reachability;
use crate::vpn::testkit::scenario::Scenario;
use std::time::Duration;

/// Ordered record of observed lifecycle events.
#[derive(Debug, Default, Clone)]
pub struct Timeline {
    events: Vec<LifecycleEvent>,
}

impl Timeline {
    /// All observed events, in order.
    pub fn events(&self) -> &[LifecycleEvent] {
        &self.events
    }

    /// Append an observed event.
    fn push(&mut self, event: LifecycleEvent) {
        self.events.push(event);
    }

    /// Whether the timeline contains the given event.
    ///
    /// Matching is by variant (label), so callers can assert that e.g. a
    /// `Connected` or `Failed { Authentication }` occurred without spelling out
    /// the exact IP/device/detail payload.
    pub fn contains(&self, event: &LifecycleEvent) -> bool {
        self.events.iter().any(|e| events_match(e, event))
    }

    /// Assert a specific event was observed.
    ///
    /// # Panics
    /// Panics with the full timeline if the event never occurred.
    pub fn assert_reached(&self, event: &LifecycleEvent) {
        assert!(
            self.contains(event),
            "expected event {:?} was never observed.\nTimeline: {}",
            event,
            self.render()
        );
    }

    /// Assert an event was NEVER observed.
    ///
    /// # Panics
    /// Panics with the full timeline if the event did occur.
    pub fn assert_never(&self, event: &LifecycleEvent) {
        assert!(
            !self.contains(event),
            "event {:?} was observed but should never occur.\nTimeline: {}",
            event,
            self.render()
        );
    }

    /// Assert that `expected` appears as an ordered (not necessarily
    /// contiguous) sub-sequence of the observed timeline.
    ///
    /// # Panics
    /// Panics with the expected vs. actual timeline on mismatch.
    pub fn assert_subsequence(&self, expected: &[LifecycleEvent]) {
        let mut idx = 0usize;
        for actual in &self.events {
            if idx < expected.len() && events_match(actual, &expected[idx]) {
                idx += 1;
            }
        }
        assert!(
            idx == expected.len(),
            "expected ordered subsequence was not found.\nExpected: {}\nActual:   {}",
            render_events(expected),
            self.render()
        );
    }

    /// Render the timeline labels for diagnostics.
    pub fn render(&self) -> String {
        render_events(&self.events)
    }
}

/// Compare two events for matching.
///
/// Payload-bearing variants match by variant so assertions can be written
/// without spelling out exact IPs/devices (e.g. assert a `Connected` happened
/// regardless of address). The one meaningful exception is `Failed`, where the
/// `kind` is significant (an auth failure is not a network failure), so it is
/// compared too.
fn events_match(actual: &LifecycleEvent, expected: &LifecycleEvent) -> bool {
    match (actual, expected) {
        (LifecycleEvent::Failed { kind: a, .. }, LifecycleEvent::Failed { kind: b, .. }) => a == b,
        _ => actual.label() == expected.label(),
    }
}

fn render_events(events: &[LifecycleEvent]) -> String {
    let labels: Vec<&str> = events.iter().map(|e| e.label()).collect();
    format!("[{}]", labels.join(" -> "))
}

/// Generic, backend-agnostic test harness.
pub struct TestHarness<B: VpnBackend> {
    backend: B,
}

impl<B: VpnBackend> TestHarness<B> {
    /// Wrap a backend.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Access the backend (e.g. to assert `is_alive()` after a run).
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Run a scenario and return the recorded [`Timeline`].
    ///
    /// The connection lifecycle is consumed from the backend's event stream.
    /// After connecting, the harness polls the scenario's [`NetworkActor`] for
    /// the derived budget, synthesizing `HealthDegraded`/`Reconnecting`/
    /// `Connected` transitions when reachability changes — modelling how the
    /// health checker + reconnection logic would react, without any real
    /// network. Finally, if the scenario asks to disconnect, the backend is
    /// torn down.
    pub async fn run(&mut self, scenario: Scenario) -> Timeline {
        let mut timeline = Timeline::default();
        let mut network = scenario.network.clone();

        // 1. Drive the connection lifecycle from the backend.
        let mut last_link: Option<LifecycleEvent> = None;
        match self.backend.connect(scenario.credentials.clone()) {
            Ok(mut rx) => {
                // Bound the wait so a misbehaving backend can't hang the suite.
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                        Ok(Some(event)) => {
                            if matches!(event, LifecycleEvent::Connected { .. }) {
                                last_link = Some(event.clone());
                            }
                            let terminal = event.is_terminal();
                            timeline.push(event);
                            if terminal {
                                break;
                            }
                        }
                        Ok(None) => break, // stream closed
                        Err(_) => {
                            timeline.push(LifecycleEvent::Failed {
                                kind: crate::vpn::backend::FailureKind::ScriptExhausted,
                                detail: "backend produced no terminal event in time".into(),
                            });
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                timeline.push(LifecycleEvent::Failed {
                    kind: crate::vpn::backend::FailureKind::Backend,
                    detail: e.to_string(),
                });
                return timeline;
            }
        }

        // Only run the network/reconnection phase if we actually connected.
        let connected = timeline
            .events()
            .iter()
            .any(|e| matches!(e, LifecycleEvent::Connected { .. }));

        if connected {
            // 2. Poll the network for the derived budget, reacting to drops.
            let mut currently_healthy = true;
            let mut attempt = 0u32;
            for _ in 0..scenario.poll_budget {
                match network.poll() {
                    Reachability::Up => {
                        if !currently_healthy {
                            // Recovery: model a reconnection cycle.
                            attempt += 1;
                            timeline.push(LifecycleEvent::Reconnecting { attempt });
                            if let Some(link) = &last_link {
                                timeline.push(link.clone());
                            } else {
                                timeline.push(LifecycleEvent::Connected {
                                    ip: "0.0.0.0".parse().unwrap(),
                                    device: "tun0".into(),
                                });
                            }
                            currently_healthy = true;
                        }
                    }
                    Reachability::Down => {
                        if currently_healthy {
                            timeline.push(LifecycleEvent::HealthDegraded);
                            currently_healthy = false;
                        }
                    }
                }
            }
        }

        // 3. Disconnect if requested by the scenario.
        let wants_disconnect = scenario
            .steps
            .iter()
            .any(|s| matches!(s, crate::vpn::testkit::scenario::ScenarioStep::Disconnect));
        if wants_disconnect {
            let _ = self.backend.disconnect();
        }

        timeline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::backend::DisconnectReason;

    fn ev_connecting() -> LifecycleEvent {
        LifecycleEvent::Connecting
    }
    fn ev_connected() -> LifecycleEvent {
        LifecycleEvent::Connected {
            ip: "10.0.0.1".parse().unwrap(),
            device: "tun0".into(),
        }
    }

    #[test]
    fn subsequence_matches_by_label() {
        let mut tl = Timeline::default();
        tl.push(ev_connecting());
        tl.push(LifecycleEvent::Authenticating);
        tl.push(ev_connected());
        // Connected matches regardless of exact ip/device.
        tl.assert_subsequence(&[
            LifecycleEvent::Connecting,
            LifecycleEvent::Connected {
                ip: "0.0.0.0".parse().unwrap(),
                device: "whatever".into(),
            },
        ]);
    }

    #[test]
    #[should_panic(expected = "subsequence")]
    fn subsequence_out_of_order_panics() {
        let mut tl = Timeline::default();
        tl.push(ev_connected());
        tl.push(ev_connecting());
        tl.assert_subsequence(&[LifecycleEvent::Connecting, ev_connected()]);
    }

    #[test]
    fn assert_never_passes_when_absent() {
        let mut tl = Timeline::default();
        tl.push(ev_connecting());
        tl.assert_never(&LifecycleEvent::Disconnected {
            reason: DisconnectReason::UserRequested,
        });
    }
}
