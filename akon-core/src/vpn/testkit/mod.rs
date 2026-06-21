//! Test actors framework: simulated backend + in-memory actors.
//!
//! This module provides everything needed to exercise akon's real-world
//! connection behavior **offline** — no root, no real `openconnect`, no real
//! network, and zero impact on the developer's internet access.
//!
//! ## Strategic purpose
//!
//! Beyond convenient testing, this framework is the **migration safety net for
//! removing the `openconnect` dependency**. Scenarios are written against the
//! backend-agnostic [`crate::vpn::backend::VpnBackend`] boundary, so the exact
//! same scenario suite that validates today's openconnect backend will later
//! validate a native, dependency-free backend — letting that replacement be
//! developed test-first and proven equivalent before it becomes the default.
//!
//! ## Building blocks
//!
//! - [`server_actor::VpnServerActor`]: scripts a backend-agnostic lifecycle.
//! - [`sim_backend::SimulatedBackend`]: an in-memory [`VpnBackend`].
//! - [`network_actor::NetworkActor`]: controls reachability over time.
//! - [`scenario::ScenarioBuilder`]: declarative scenario authoring.
//! - [`harness::TestHarness`]: generic over any backend; records a [`harness::Timeline`].
//!
//! [`VpnBackend`]: crate::vpn::backend::VpnBackend

pub mod f5_server_actor;
pub mod fake_dns;
pub mod fake_tun;
pub mod harness;
pub mod network_actor;
pub mod scenario;
pub mod server_actor;
pub mod sim_backend;
pub mod transport;

// Convenience re-exports for ergonomic test imports.
pub use f5_server_actor::{F5ServerActor, F5ServerScript};
pub use fake_dns::{FakeDns, FakeDnsHandle};
pub use fake_tun::{FakeTun, FakeTunHandle};
pub use harness::{TestHarness, Timeline};
pub use network_actor::{NetworkActor, Reachability};
pub use scenario::{Scenario, ScenarioBuilder, ScenarioStep};
pub use server_actor::{ServerStep, VpnServerActor};
pub use sim_backend::{FakeTunnelRegistry, SimulatedBackend, TunnelState};
pub use transport::MemoryTransport;
