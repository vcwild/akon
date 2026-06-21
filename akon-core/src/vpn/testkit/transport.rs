//! In-memory duplex [`Transport`] for testing the native backends offline.
//!
//! [`MemoryTransport::pair`] returns two connected endpoints; bytes written to
//! one are readable from the other. The fake F5 server actor drives one end
//! while [`crate::vpn::f5::NativeF5Backend`] drives the other — no real TLS,
//! TCP, or network involved.

use crate::vpn::transport::Transport;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;

/// Shared byte buffer for one direction of the duplex.
#[derive(Default)]
struct Pipe {
    buf: VecDeque<u8>,
}

#[derive(Clone)]
struct Channel {
    pipe: Arc<Mutex<Pipe>>,
    /// Set when the writer end is closed (explicitly or dropped). An atomic so
    /// it can be flipped synchronously from `Drop` without awaiting a lock —
    /// this is what guarantees a blocked `recv` on the peer observes EOF
    /// instead of hanging forever.
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Channel {
    fn new() -> Self {
        Self {
            pipe: Arc::new(Mutex::new(Pipe::default())),
            closed: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    async fn write(&self, data: &[u8]) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"));
        }
        {
            let mut p = self.pipe.lock().await;
            p.buf.extend(data.iter().copied());
        }
        self.notify.notify_waiters();
        Ok(())
    }

    async fn read(&self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            // Register for notification BEFORE checking state so we never miss a
            // wake that happens between the check and the await.
            let notified = self.notify.notified();

            {
                let mut p = self.pipe.lock().await;
                if !p.buf.is_empty() {
                    let n = out.len().min(p.buf.len());
                    for slot in out.iter_mut().take(n) {
                        *slot = p.buf.pop_front().expect("buffer non-empty");
                    }
                    return Ok(n);
                }
            }
            // Buffer empty: if the writer has closed, signal EOF.
            if self.closed.load(Ordering::Acquire) {
                return Ok(0);
            }

            notified.await;
        }
    }

    fn close_sync(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
}

/// One endpoint of an in-memory full-duplex byte stream.
pub struct MemoryTransport {
    /// Channel this endpoint reads from.
    inbound: Channel,
    /// Channel this endpoint writes to.
    outbound: Channel,
}

impl MemoryTransport {
    /// Create a connected pair of endpoints `(a, b)`.
    pub fn pair() -> (MemoryTransport, MemoryTransport) {
        let a2b = Channel::new();
        let b2a = Channel::new();
        let a = MemoryTransport {
            inbound: b2a.clone(),
            outbound: a2b.clone(),
        };
        let b = MemoryTransport {
            inbound: a2b,
            outbound: b2a,
        };
        (a, b)
    }
}

#[async_trait]
impl Transport for MemoryTransport {
    async fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.outbound.write(data).await
    }

    async fn recv(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inbound.read(buf).await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.outbound.close_sync();
        Ok(())
    }
}

impl Drop for MemoryTransport {
    /// Dropping an endpoint closes its outbound channel so the peer's pending
    /// `recv` observes EOF (`Ok(0)`) rather than blocking forever. This is what
    /// makes actor loops terminate deterministically in tests.
    fn drop(&mut self) {
        self.outbound.close_sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pair_round_trips_bytes() {
        let (mut a, mut b) = MemoryTransport::pair();
        a.send(b"hello").await.unwrap();
        let mut buf = [0u8; 16];
        let n = b.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[tokio::test]
    async fn close_yields_zero_read() {
        let (mut a, mut b) = MemoryTransport::pair();
        a.close().await.unwrap();
        let mut buf = [0u8; 4];
        let n = b.recv(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn bidirectional() {
        let (mut a, mut b) = MemoryTransport::pair();
        a.send(b"ping").await.unwrap();
        b.send(b"pong").await.unwrap();
        let mut buf = [0u8; 8];
        let n = b.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"ping");
        let n = a.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"pong");
    }
}
