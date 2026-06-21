//! Standalone F5 test server (TLS) — the workload run inside a Podman container
//! for real-host integration testing of the native F5 backend.
//!
//! It generates a self-signed certificate (SAN from `AKON_F5_SAN`, default
//! `127.0.0.1`), writes the certificate PEM to `AKON_F5_CERT_OUT` (so the client
//! can trust it), listens for TLS connections on `AKON_F5_LISTEN`
//! (default `0.0.0.0:8443`), and serves the real F5 protocol via
//! [`akon_core::vpn::testkit::f5_server_actor::F5ServerActor`] over each
//! accepted TLS stream.
//!
//! This binary is only compiled with the `test-actors` feature, so it never
//! ships in release builds.
//!
//! Env vars:
//! - `AKON_F5_LISTEN`   — bind address (default `0.0.0.0:8443`)
//! - `AKON_F5_SAN`      — certificate SAN, an IP or DNS name (default `127.0.0.1`)
//! - `AKON_F5_CERT_OUT` — path to write the server cert PEM (default `/certs/server.pem`)
//! - `AKON_F5_ASSIGNED_IP` — IPv4 the server assigns the client (default `10.20.30.40`)

use std::net::IpAddr;
use std::sync::Arc;

use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::transport::Transport;
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// Adapter so the server side of a real TLS stream satisfies `Transport`.
struct ServerTlsTransport {
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
}

#[async_trait]
impl Transport for ServerTlsTransport {
    async fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(data).await?;
        self.stream.flush().await
    }
    async fn recv(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf).await
    }
    async fn close(&mut self) -> std::io::Result<()> {
        self.stream.shutdown().await
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen = env_or("AKON_F5_LISTEN", "0.0.0.0:8443");
    let san = env_or("AKON_F5_SAN", "127.0.0.1");
    let cert_out = env_or("AKON_F5_CERT_OUT", "/certs/server.pem");
    let assigned_ip = env_or("AKON_F5_ASSIGNED_IP", "10.20.30.40");

    // Generate a self-signed certificate with the requested SAN(s) (comma-
    // separated; each entry an IP or DNS name). Always include loopback so the
    // host can also reach the published port.
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())?;
    let mut sans: Vec<String> = san.split(',').map(|s| s.trim().to_string()).collect();
    if !sans.iter().any(|s| s == "127.0.0.1") {
        sans.push("127.0.0.1".to_string());
    }
    for entry in &sans {
        match entry.parse::<IpAddr>() {
            Ok(ip) => params.subject_alt_names.push(rcgen::SanType::IpAddress(ip)),
            Err(_) => params
                .subject_alt_names
                .push(rcgen::SanType::DnsName(entry.clone().try_into()?)),
        }
    }
    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    // Write the cert PEM so the client can trust it.
    let cert_pem = cert.pem();
    if let Some(parent) = std::path::Path::new(&cert_out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&cert_out, &cert_pem)?;
    eprintln!("f5_test_server: wrote cert to {cert_out}");

    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::try_from(key_pair.serialize_der())?;
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    let assigned: [u8; 4] = {
        let ip: std::net::Ipv4Addr = assigned_ip.parse()?;
        ip.octets()
    };
    let script = F5ServerScript {
        assigned_ip: assigned,
        ..F5ServerScript::default()
    };

    let listener = TcpListener::bind(&listen).await?;
    eprintln!("f5_test_server: listening on {listen} (SAN={san})");

    loop {
        let (tcp, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let script = script.clone();
        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                Ok(tls) => {
                    eprintln!("f5_test_server: TLS session from {peer}");
                    let mut transport = ServerTlsTransport { stream: tls };
                    F5ServerActor::new(script).run(&mut transport).await;
                }
                Err(e) => eprintln!("f5_test_server: TLS handshake failed: {e}"),
            }
        });
    }
}
