//! Real TLS-over-TCP [`Transport`] for the native F5 backend (production path).
//!
//! This is the concrete transport used against a live F5 server. It is kept
//! deliberately thin: connect a TCP socket, perform a rustls TLS handshake, and
//! expose the duplex byte stream through the [`Transport`] seam. All protocol
//! logic lives above the seam and is validated offline by the test actors
//! framework; this module is the small, real-I/O adapter the same logic runs
//! over in production (and in the *real* end-to-end test against a local TLS
//! server).
//!
//! Bounded I/O is the caller's responsibility: the F5 backend wraps the whole
//! handshake and the PPP loop in `tokio::time::timeout`, so a stalled real
//! socket fails deterministically instead of hanging.

use crate::vpn::transport::{Transport, TransportFactory};
use async_trait::async_trait;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};

/// A real TLS-over-TCP transport.
pub struct TlsTransport {
    stream: TlsStream<TcpStream>,
}

impl TlsTransport {
    /// Connect to `host:port` and perform a TLS handshake validating against the
    /// webpki root store (production trust).
    pub async fn connect(host: &str, port: u16) -> io::Result<Self> {
        let roots = webpki_roots_store();
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self::connect_with_config(host, port, Arc::new(config)).await
    }

    /// Connect using a caller-supplied [`ClientConfig`].
    ///
    /// This is the seam the **real** end-to-end test uses to trust a local,
    /// self-signed server certificate without weakening production trust.
    pub async fn connect_with_config(
        host: &str,
        port: u16,
        config: Arc<ClientConfig>,
    ) -> io::Result<Self> {
        let tcp = TcpStream::connect((host, port)).await?;
        tcp.set_nodelay(true).ok();
        let connector = TlsConnector::from(config);
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid server name"))?;
        let stream = connector.connect(server_name, tcp).await?;
        Ok(Self { stream })
    }
}

/// Build a root store from the bundled webpki roots.
fn webpki_roots_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// A [`TransportFactory`] that opens fresh TLS connections to a fixed host:port.
///
/// Used by the auth/config phase so it can reconnect when the server closes the
/// connection between requests (the common real-F5 behaviour).
pub struct TlsTransportFactory {
    host: String,
    port: u16,
    config: Arc<ClientConfig>,
}

impl TlsTransportFactory {
    /// Create a factory connecting to `host:port` with production webpki trust.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        let config = ClientConfig::builder()
            .with_root_certificates(webpki_roots_store())
            .with_no_client_auth();
        Self {
            host: host.into(),
            port,
            config: Arc::new(config),
        }
    }

    /// Create a factory with a caller-supplied client config (used by tests to
    /// trust a self-signed/local cert).
    pub fn with_config(host: impl Into<String>, port: u16, config: Arc<ClientConfig>) -> Self {
        Self {
            host: host.into(),
            port,
            config,
        }
    }
}

#[async_trait]
impl TransportFactory for TlsTransportFactory {
    async fn connect(&self) -> io::Result<Box<dyn Transport>> {
        let t = TlsTransport::connect_with_config(&self.host, self.port, Arc::clone(&self.config))
            .await?;
        Ok(Box::new(t))
    }
}

#[async_trait]
impl Transport for TlsTransport {
    async fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(data).await?;
        self.stream.flush().await
    }

    async fn recv(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf).await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.stream.shutdown().await
    }
}
