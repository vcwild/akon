//! In-memory fake [`TunDevice`] for testing the native F5 data plane offline.
//!
//! Records the [`TunConfig`] applied and every packet written "into the OS"
//! (i.e. received from the tunnel), and lets a test inject packets "from the OS"
//! (i.e. to be sent over the tunnel). No real `/dev/net/tun`, no root.

use crate::vpn::transport::{TunConfig, TunDevice};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Shared, observable state of the fake TUN device.
#[derive(Default)]
struct Inner {
    /// The configuration applied via [`TunDevice::configure`].
    config: Option<TunConfig>,
    /// Packets written to the device (received from the tunnel, destined for OS).
    to_os: Vec<Vec<u8>>,
    /// Packets queued to be read by the device (from OS, destined for tunnel).
    from_os: VecDeque<Vec<u8>>,
}

/// A handle to inspect/drive a [`FakeTun`] from a test.
#[derive(Clone, Default)]
pub struct FakeTunHandle {
    inner: Arc<Mutex<Inner>>,
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl FakeTunHandle {
    /// The configuration the backend applied to the interface, if any.
    pub fn applied_config(&self) -> Option<TunConfig> {
        self.inner.lock().expect("poisoned").config.clone()
    }

    /// All packets the backend delivered to the OS (decapsulated from the tunnel).
    pub fn packets_to_os(&self) -> Vec<Vec<u8>> {
        self.inner.lock().expect("poisoned").to_os.clone()
    }

    /// Queue an outbound packet as if the OS produced it for the tunnel.
    pub fn inject_from_os(&self, packet: Vec<u8>) {
        self.inner
            .lock()
            .expect("poisoned")
            .from_os
            .push_back(packet);
        self.notify.notify_waiters();
    }

    /// Close the device so the backend's read loop observes EOF and stops.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
}

/// In-memory TUN device. Construct with [`FakeTun::new`] and keep the returned
/// [`FakeTunHandle`] to drive/inspect it.
pub struct FakeTun {
    inner: Arc<Mutex<Inner>>,
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl FakeTun {
    /// Create a fake TUN device and a handle to it.
    pub fn new() -> (FakeTun, FakeTunHandle) {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let closed = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let handle = FakeTunHandle {
            inner: Arc::clone(&inner),
            closed: Arc::clone(&closed),
            notify: Arc::clone(&notify),
        };
        (
            FakeTun {
                inner,
                closed,
                notify,
            },
            handle,
        )
    }
}

#[async_trait]
impl TunDevice for FakeTun {
    async fn configure(&mut self, config: &TunConfig) -> io::Result<()> {
        self.inner.lock().expect("poisoned").config = Some(config.clone());
        Ok(())
    }

    async fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        self.inner
            .lock()
            .expect("poisoned")
            .to_os
            .push(packet.to_vec());
        Ok(())
    }

    async fn read_packet(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let notified = self.notify.notified();
            {
                let mut inner = self.inner.lock().expect("poisoned");
                if let Some(pkt) = inner.from_os.pop_front() {
                    let n = pkt.len().min(buf.len());
                    buf[..n].copy_from_slice(&pkt[..n]);
                    return Ok(n);
                }
            }
            if self.closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_config_and_to_os_packets() {
        let (mut tun, handle) = FakeTun::new();
        let cfg = TunConfig {
            ipv4: Some("10.0.0.2".into()),
            mtu: Some(1400),
            ..Default::default()
        };
        tun.configure(&cfg).await.unwrap();
        tun.write_packet(&[1, 2, 3]).await.unwrap();
        assert_eq!(
            handle.applied_config().and_then(|c| c.ipv4),
            Some("10.0.0.2".into())
        );
        assert_eq!(handle.packets_to_os(), vec![vec![1, 2, 3]]);
    }

    #[tokio::test]
    async fn read_returns_injected_then_eof_on_close() {
        let (mut tun, handle) = FakeTun::new();
        handle.inject_from_os(vec![9, 9]);
        let mut buf = [0u8; 8];
        let n = tun.read_packet(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], &[9, 9]);
        handle.close();
        let n = tun.read_packet(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }
}
