//! Recording fake [`DnsApplier`] for testing DNS application offline.
//!
//! Captures the `apply`/`revert` calls (interface + config) so a test can assert
//! that the native backend would apply the negotiated DNS servers/domains —
//! without touching the host resolver.

use crate::vpn::f5::dns::DnsApplier;
use crate::vpn::transport::TunConfig;
use std::sync::{Arc, Mutex};

/// Shared record of DNS operations.
#[derive(Debug, Default, Clone)]
pub struct DnsRecord {
    /// `(iface, config)` pairs passed to `apply`.
    pub applied: Vec<(String, TunConfig)>,
    /// Interfaces passed to `revert`.
    pub reverted: Vec<String>,
}

/// A handle to inspect what a [`FakeDns`] recorded.
#[derive(Debug, Default, Clone)]
pub struct FakeDnsHandle {
    inner: Arc<Mutex<DnsRecord>>,
}

impl FakeDnsHandle {
    /// Snapshot of the recorded operations.
    pub fn record(&self) -> DnsRecord {
        self.inner.lock().expect("poisoned").clone()
    }

    /// Whether DNS was applied with the given servers (in any apply call).
    pub fn applied_servers(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("poisoned")
            .applied
            .iter()
            .flat_map(|(_, c)| c.dns.clone())
            .collect()
    }
}

/// A DNS applier that records instead of touching the host.
#[derive(Default)]
pub struct FakeDns {
    inner: Arc<Mutex<DnsRecord>>,
}

impl FakeDns {
    /// Create a fake DNS applier and a handle to inspect it.
    pub fn new() -> (FakeDns, FakeDnsHandle) {
        let inner = Arc::new(Mutex::new(DnsRecord::default()));
        (
            FakeDns {
                inner: Arc::clone(&inner),
            },
            FakeDnsHandle { inner },
        )
    }
}

impl DnsApplier for FakeDns {
    fn apply(&mut self, iface: &str, config: &TunConfig) -> std::io::Result<()> {
        self.inner
            .lock()
            .expect("poisoned")
            .applied
            .push((iface.to_string(), config.clone()));
        Ok(())
    }

    fn revert(&mut self, iface: &str) -> std::io::Result<()> {
        self.inner
            .lock()
            .expect("poisoned")
            .reverted
            .push(iface.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_apply_and_revert() {
        let (mut dns, handle) = FakeDns::new();
        let cfg = TunConfig {
            dns: vec!["10.0.0.53".into()],
            ..Default::default()
        };
        dns.apply("tun0", &cfg).unwrap();
        dns.revert("tun0").unwrap();
        let rec = handle.record();
        assert_eq!(rec.applied.len(), 1);
        assert_eq!(rec.reverted, vec!["tun0".to_string()]);
        assert_eq!(handle.applied_servers(), vec!["10.0.0.53".to_string()]);
    }
}
