//! Simulated VPN backend + fake tunnel registry.
//!
//! [`SimulatedBackend`] implements the durable [`VpnBackend`] boundary entirely
//! in memory. It is driven by a [`VpnServerActor`] script and tracks the
//! "tunnel" via a [`FakeTunnelRegistry`]. No real process, root, or network is
//! ever involved — which is the whole point: the same scenarios that drive this
//! backend will later drive a native backend, proving behavioral equivalence
//! before `openconnect` is removed.

use crate::vpn::backend::TermSignal;
use crate::vpn::backend::{
    BackendError, ConnectionHandle, Credentials, LifecycleEvent, VpnBackend,
};
use crate::vpn::testkit::server_actor::VpnServerActor;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{self, UnboundedReceiver};

/// State of a simulated tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    /// Live and usable.
    Alive,
    /// Graceful teardown requested but not yet honored.
    Terminating,
    /// Fully torn down.
    Terminated,
}

/// A single simulated tunnel/connection.
#[derive(Debug, Clone)]
struct SimTunnel {
    state: TunnelState,
    /// When true, the tunnel ignores graceful (SIGTERM-equivalent) signals and
    /// only terminates on a forced signal — used to exercise the
    /// graceful→forced escalation path.
    ignores_graceful: bool,
}

/// In-memory registry of simulated tunnels.
///
/// Models what *any* backend must track (a live connection handle and its
/// teardown), independent of openconnect PIDs.
#[derive(Debug, Clone, Default)]
pub struct FakeTunnelRegistry {
    inner: Arc<Mutex<HashMap<u64, SimTunnel>>>,
    next: Arc<AtomicU64>,
}

impl FakeTunnelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            next: Arc::new(AtomicU64::new(1000)),
        }
    }

    /// Register a new alive tunnel and return its handle.
    pub fn register(&self) -> ConnectionHandle {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.inner.lock().expect("registry poisoned").insert(
            id,
            SimTunnel {
                state: TunnelState::Alive,
                ignores_graceful: false,
            },
        );
        ConnectionHandle(id)
    }

    /// Make a tunnel ignore graceful signals (to test forced escalation).
    pub fn set_ignores_graceful(&self, handle: ConnectionHandle, value: bool) {
        if let Some(t) = self
            .inner
            .lock()
            .expect("registry poisoned")
            .get_mut(&handle.0)
        {
            t.ignores_graceful = value;
        }
    }

    /// Whether the tunnel is currently alive.
    pub fn is_alive(&self, handle: ConnectionHandle) -> bool {
        self.inner
            .lock()
            .expect("registry poisoned")
            .get(&handle.0)
            .map(|t| t.state == TunnelState::Alive)
            .unwrap_or(false)
    }

    /// Current tunnel state, if known.
    pub fn state(&self, handle: ConnectionHandle) -> Option<TunnelState> {
        self.inner
            .lock()
            .expect("registry poisoned")
            .get(&handle.0)
            .map(|t| t.state)
    }

    /// Deliver a termination signal to a tunnel.
    ///
    /// - `Forced` always terminates immediately.
    /// - `Graceful` terminates immediately unless the tunnel ignores graceful
    ///   signals, in which case it transitions to `Terminating` and awaits a
    ///   forced signal.
    pub fn signal(&self, handle: ConnectionHandle, sig: TermSignal) {
        let mut guard = self.inner.lock().expect("registry poisoned");
        if let Some(t) = guard.get_mut(&handle.0) {
            match sig {
                TermSignal::Forced => t.state = TunnelState::Terminated,
                TermSignal::Graceful => {
                    if t.ignores_graceful {
                        t.state = TunnelState::Terminating;
                    } else {
                        t.state = TunnelState::Terminated;
                    }
                }
            }
        }
    }
}

/// Fully in-memory [`VpnBackend`] implementation for tests.
pub struct SimulatedBackend {
    server: Option<VpnServerActor>,
    registry: FakeTunnelRegistry,
    handle: Arc<Mutex<Option<ConnectionHandle>>>,
}

impl SimulatedBackend {
    /// Create a simulated backend driven by the given server actor.
    pub fn new(server: VpnServerActor) -> Self {
        Self {
            server: Some(server),
            registry: FakeTunnelRegistry::new(),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Access the underlying registry (to inspect tunnel state in assertions).
    pub fn registry(&self) -> FakeTunnelRegistry {
        self.registry.clone()
    }
}

impl VpnBackend for SimulatedBackend {
    fn connect(
        &mut self,
        _credentials: Credentials,
    ) -> Result<UnboundedReceiver<LifecycleEvent>, BackendError> {
        let mut server = self.server.take().ok_or(BackendError::AlreadyConnected)?;

        let (tx, rx) = mpsc::unbounded_channel();
        let registry = self.registry.clone();
        let handle_slot = Arc::clone(&self.handle);

        tokio::spawn(async move {
            let mut tunnel_handle: Option<ConnectionHandle> = None;

            while let Some(event) = server.next_event() {
                // On the first sign of an established link, register a live
                // tunnel and record its handle.
                match &event {
                    LifecycleEvent::LinkUp { .. } | LifecycleEvent::Connected { .. } => {
                        if tunnel_handle.is_none() {
                            let h = registry.register();
                            tunnel_handle = Some(h);
                            *handle_slot.lock().expect("handle lock poisoned") = Some(h);
                        }
                    }
                    LifecycleEvent::Disconnected { .. } | LifecycleEvent::Failed { .. } => {
                        if let Some(h) = tunnel_handle {
                            registry.signal(h, TermSignal::Forced);
                        }
                    }
                    _ => {}
                }

                let terminal = event.is_terminal();
                if tx.send(event).is_err() {
                    break;
                }
                if terminal {
                    break;
                }
            }
        });

        Ok(rx)
    }

    fn disconnect(&mut self) -> Result<(), BackendError> {
        let handle = *self.handle.lock().expect("handle lock poisoned");
        if let Some(h) = handle {
            // Graceful first; if the tunnel honors it, it terminates. Otherwise
            // escalate to forced (mirrors production SIGTERM→SIGKILL).
            self.registry.signal(h, TermSignal::Graceful);
            if self.registry.state(h) == Some(TunnelState::Terminating) {
                self.registry.signal(h, TermSignal::Forced);
            }
        }
        Ok(())
    }

    fn is_alive(&self) -> bool {
        let handle = *self.handle.lock().expect("handle lock poisoned");
        handle.map(|h| self.registry.is_alive(h)).unwrap_or(false)
    }

    fn handle(&self) -> Option<ConnectionHandle> {
        *self.handle.lock().expect("handle lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_register_is_alive() {
        let reg = FakeTunnelRegistry::new();
        let h = reg.register();
        assert!(reg.is_alive(h));
    }

    #[test]
    fn forced_signal_terminates() {
        let reg = FakeTunnelRegistry::new();
        let h = reg.register();
        reg.signal(h, TermSignal::Forced);
        assert!(!reg.is_alive(h));
        assert_eq!(reg.state(h), Some(TunnelState::Terminated));
    }

    #[test]
    fn graceful_honored_terminates() {
        let reg = FakeTunnelRegistry::new();
        let h = reg.register();
        reg.signal(h, TermSignal::Graceful);
        assert!(!reg.is_alive(h));
    }

    #[test]
    fn graceful_ignored_requires_forced() {
        let reg = FakeTunnelRegistry::new();
        let h = reg.register();
        reg.set_ignores_graceful(h, true);
        reg.signal(h, TermSignal::Graceful);
        assert_eq!(reg.state(h), Some(TunnelState::Terminating));
        assert!(!reg.is_alive(h)); // not alive while terminating
        reg.signal(h, TermSignal::Forced);
        assert_eq!(reg.state(h), Some(TunnelState::Terminated));
    }
}
