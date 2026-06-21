//! Declarative, backend-independent scenarios.
//!
//! A [`Scenario`] describes a real-world situation as data: what the server
//! does (via the backend it is run against) and how the network behaves over
//! time (via a [`NetworkActor`]). The [`ScenarioBuilder`] provides a fluent API
//! so a new real-world regression test reads like prose.
//!
//! Scenarios are intentionally **backend-independent**: the same scenario can
//! be executed against the simulated backend today and a native backend
//! tomorrow (see [`crate::vpn::testkit::harness::TestHarness`]).

use crate::vpn::backend::Credentials;
use crate::vpn::testkit::network_actor::NetworkActor;

/// A high-level step in a scenario, expressing developer intent.
#[derive(Debug, Clone)]
pub enum ScenarioStep {
    /// Establish the connection (drives the backend's `connect`).
    Connect,
    /// Expect the link to remain healthy for `polls` network polls.
    StayHealthy(usize),
    /// Drop network connectivity for `polls` polls (then recover).
    DropNetwork(usize),
    /// Expect the link to recover via reconnection.
    ExpectReconnect,
    /// Disconnect the connection.
    Disconnect,
}

/// A complete, runnable scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Ordered intent steps.
    pub steps: Vec<ScenarioStep>,
    /// Network behavior over time.
    pub network: NetworkActor,
    /// Credentials handed to the backend on connect.
    pub credentials: Credentials,
    /// Number of network polls the harness performs after connecting (drives
    /// health-based reconnection). Derived from the steps when built.
    pub poll_budget: usize,
}

/// Fluent builder for [`Scenario`]s.
#[derive(Debug, Clone)]
pub struct ScenarioBuilder {
    steps: Vec<ScenarioStep>,
    network: Option<NetworkActor>,
    credentials: Credentials,
}

impl Default for ScenarioBuilder {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            network: None,
            credentials: Credentials::new("test-user", "test-pass"),
        }
    }
}

impl ScenarioBuilder {
    /// Start a new scenario.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use specific credentials.
    pub fn credentials(mut self, username: &str, password: &str) -> Self {
        self.credentials = Credentials::new(username, password);
        self
    }

    /// Override the network actor explicitly.
    pub fn network(mut self, network: NetworkActor) -> Self {
        self.network = Some(network);
        self
    }

    /// Establish the connection.
    pub fn connect(mut self) -> Self {
        self.steps.push(ScenarioStep::Connect);
        self
    }

    /// Stay healthy for `polls` polls.
    pub fn stay_healthy(mut self, polls: usize) -> Self {
        self.steps.push(ScenarioStep::StayHealthy(polls));
        self
    }

    /// Drop the network for `polls` polls.
    pub fn drop_network(mut self, polls: usize) -> Self {
        self.steps.push(ScenarioStep::DropNetwork(polls));
        self
    }

    /// Expect a reconnection to recover the link.
    pub fn expect_reconnect(mut self) -> Self {
        self.steps.push(ScenarioStep::ExpectReconnect);
        self
    }

    /// Disconnect.
    pub fn disconnect(mut self) -> Self {
        self.steps.push(ScenarioStep::Disconnect);
        self
    }

    /// Finalize the scenario.
    ///
    /// When no explicit network actor was provided, one is derived from the
    /// `StayHealthy`/`DropNetwork` steps so the timeline is fully determined by
    /// the declarative description.
    pub fn build(self) -> Scenario {
        // Derive a network script + poll budget from the steps if not set.
        let mut derived: Vec<bool> = Vec::new();
        let mut expects_reconnect = false;
        for step in &self.steps {
            match step {
                ScenarioStep::StayHealthy(n) => derived.extend(std::iter::repeat(true).take(*n)),
                ScenarioStep::DropNetwork(n) => derived.extend(std::iter::repeat(false).take(*n)),
                ScenarioStep::ExpectReconnect => expects_reconnect = true,
                _ => {}
            }
        }

        // When the scenario expects a reconnect, append a recovery poll so the
        // network returns to healthy and the harness can observe the
        // Reconnecting -> Connected cycle. The poll budget must cover it.
        if expects_reconnect {
            derived.push(true);
        }

        let poll_budget = derived.len();
        let network = self.network.unwrap_or_else(|| {
            if derived.is_empty() {
                NetworkActor::reachable()
            } else {
                NetworkActor::script(derived.clone())
            }
        });

        Scenario {
            steps: self.steps,
            network,
            credentials: self.credentials,
            poll_budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_records_steps_in_order() {
        let scenario = ScenarioBuilder::new()
            .connect()
            .stay_healthy(2)
            .drop_network(2)
            .expect_reconnect()
            .disconnect()
            .build();

        assert!(matches!(scenario.steps[0], ScenarioStep::Connect));
        assert!(matches!(scenario.steps[1], ScenarioStep::StayHealthy(2)));
        assert!(matches!(scenario.steps[2], ScenarioStep::DropNetwork(2)));
        assert!(matches!(scenario.steps[3], ScenarioStep::ExpectReconnect));
        assert!(matches!(scenario.steps[4], ScenarioStep::Disconnect));
    }

    #[test]
    fn poll_budget_derived_from_steps() {
        let scenario = ScenarioBuilder::new()
            .connect()
            .stay_healthy(3)
            .drop_network(2)
            .build();
        assert_eq!(scenario.poll_budget, 5);
    }
}
