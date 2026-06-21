//! Scriptable VPN server actor.
//!
//! [`VpnServerActor`] plays the role of the remote VPN server + transport,
//! driven by a script of backend-agnostic [`LifecycleEvent`]s. It never
//! performs any real I/O — it simply yields the next scripted event.
//!
//! Convenience constructors produce the common real-world shapes
//! (successful connect, authentication failure, connect-then-drop) so most
//! tests don't need to hand-write a script.

use crate::vpn::backend::{DisconnectReason, FailureKind, LifecycleEvent};
use std::collections::VecDeque;
use std::net::IpAddr;

/// A single scripted step the server actor performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStep {
    /// Emit a lifecycle event.
    Emit(LifecycleEvent),
    /// A logical delay, in milliseconds (no wall-clock sleep is performed; the
    /// harness interprets this as ordering/spacing).
    Delay(u64),
}

/// In-memory actor that yields a scripted sequence of lifecycle events.
#[derive(Debug, Default)]
pub struct VpnServerActor {
    steps: VecDeque<ServerStep>,
}

impl VpnServerActor {
    /// Create an actor with an explicit script.
    pub fn script(steps: Vec<ServerStep>) -> Self {
        Self {
            steps: steps.into(),
        }
    }

    /// Script: a fully successful connection ending in `Connected`.
    pub fn successful_connect(ip: IpAddr, device: &str) -> Self {
        Self::script(vec![
            ServerStep::Emit(LifecycleEvent::Connecting),
            ServerStep::Emit(LifecycleEvent::Authenticating),
            ServerStep::Emit(LifecycleEvent::SessionEstablished),
            ServerStep::Emit(LifecycleEvent::LinkUp {
                ip,
                device: device.to_string(),
            }),
            ServerStep::Emit(LifecycleEvent::Connected {
                ip,
                device: device.to_string(),
            }),
        ])
    }

    /// Script: authentication fails; the flow never reaches `Connected`.
    pub fn auth_failure(detail: &str) -> Self {
        Self::script(vec![
            ServerStep::Emit(LifecycleEvent::Connecting),
            ServerStep::Emit(LifecycleEvent::Authenticating),
            ServerStep::Emit(LifecycleEvent::Failed {
                kind: FailureKind::Authentication,
                detail: detail.to_string(),
            }),
        ])
    }

    /// Script: connect successfully, stay up, then the link silently drops.
    ///
    /// This emits the connect sequence followed by `HealthDegraded`, modelling
    /// a silent tunnel death that the health checker would observe.
    pub fn connect_then_drop(ip: IpAddr, device: &str) -> Self {
        Self::script(vec![
            ServerStep::Emit(LifecycleEvent::Connecting),
            ServerStep::Emit(LifecycleEvent::Authenticating),
            ServerStep::Emit(LifecycleEvent::SessionEstablished),
            ServerStep::Emit(LifecycleEvent::LinkUp {
                ip,
                device: device.to_string(),
            }),
            ServerStep::Emit(LifecycleEvent::Connected {
                ip,
                device: device.to_string(),
            }),
            ServerStep::Emit(LifecycleEvent::HealthDegraded),
            ServerStep::Emit(LifecycleEvent::Disconnected {
                reason: DisconnectReason::LinkLost,
            }),
        ])
    }

    /// Yield the next lifecycle event, skipping over logical delays.
    ///
    /// Returns `None` once the script is exhausted.
    pub fn next_event(&mut self) -> Option<LifecycleEvent> {
        while let Some(step) = self.steps.pop_front() {
            match step {
                ServerStep::Emit(event) => return Some(event),
                ServerStep::Delay(_) => continue,
            }
        }
        None
    }

    /// Whether the script has remaining steps.
    pub fn has_more(&self) -> bool {
        !self.steps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "10.0.0.5".parse().unwrap()
    }

    #[test]
    fn successful_connect_ends_in_connected() {
        let mut actor = VpnServerActor::successful_connect(ip(), "tun0");
        let mut last = None;
        while let Some(e) = actor.next_event() {
            last = Some(e);
        }
        assert_eq!(
            last,
            Some(LifecycleEvent::Connected {
                ip: ip(),
                device: "tun0".into()
            })
        );
    }

    #[test]
    fn auth_failure_ends_in_authentication_failure() {
        let mut actor = VpnServerActor::auth_failure("bad creds");
        let mut last = None;
        while let Some(e) = actor.next_event() {
            last = Some(e);
        }
        assert_eq!(
            last,
            Some(LifecycleEvent::Failed {
                kind: FailureKind::Authentication,
                detail: "bad creds".into()
            })
        );
    }

    #[test]
    fn delays_are_skipped() {
        let mut actor = VpnServerActor::script(vec![
            ServerStep::Delay(100),
            ServerStep::Emit(LifecycleEvent::Connecting),
            ServerStep::Delay(50),
            ServerStep::Emit(LifecycleEvent::Authenticating),
        ]);
        assert_eq!(actor.next_event(), Some(LifecycleEvent::Connecting));
        assert_eq!(actor.next_event(), Some(LifecycleEvent::Authenticating));
        assert_eq!(actor.next_event(), None);
    }
}
